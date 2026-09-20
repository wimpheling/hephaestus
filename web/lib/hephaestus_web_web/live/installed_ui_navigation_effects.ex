defmodule HephaestusWebWeb.InstalledUiNavigationEffects do
  @moduledoc """
  State and effects for the installed UI navigation surface.

  The finite lifecycle state keeps its retained navigation projection under
  `data`. Handoff bearers and bootstrap URLs remain transient effect results;
  they are never retained in the state or LiveView assigns.
  """

  alias HephaestusWeb.RPC.{Client, Error}
  alias HephaestusWeb.UIBrowser

  @statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  @type target :: :global | {:project, String.t()} | {:repository, String.t()}
  @type t :: %__MODULE__{
          status: atom(),
          data: map(),
          form: nil,
          error: String.t() | nil,
          cursor: String.t() | nil,
          stream_generation: non_neg_integer()
        }

  defstruct status: :initial,
            data: %{},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  @doc "Returns the standard lifecycle statuses for this effects model."
  @spec statuses() :: [atom()]
  def statuses, do: @statuses

  @doc "Creates a loading projection for one explicit organization and target."
  @spec new(%{organization_id: String.t(), target: target()}) :: t()
  def new(%{organization_id: organization_id, target: target}) do
    %__MODULE__{data: initial_data(organization_id, target)}
  end

  @doc "Reduces navigation commands and delivered effects into state transitions."
  @spec reduce(t(), term()) :: {t(), [term()]}
  def reduce(%__MODULE__{} = state, :load) do
    generation = state.stream_generation + 1

    {%{state | status: :loading, error: nil, stream_generation: generation},
     [{:load, generation}]}
  end

  def reduce(%__MODULE__{} = state, :refresh) do
    state = refresh(state)
    {state, [{:load, state.stream_generation}]}
  end

  def reduce(%__MODULE__{} = state, :load_more) do
    case state.data.next_page_token do
      token when is_binary(token) ->
        state = load_more(state)
        {state, [{:load, state.stream_generation}]}

      _missing ->
        {state, []}
    end
  end

  def reduce(%__MODULE__{} = state, :close), do: {deactivate(state), [{:close, "user_closed"}]}

  def reduce(%__MODULE__{} = state, :terminate),
    do: {terminate(state), [{:close, "terminated"}]}

  def reduce(%__MODULE__{} = state, :stale),
    do: {%{state | status: :stale}, [{:load, state.stream_generation}]}

  def reduce(%__MODULE__{} = state, :reconnecting),
    do: {%{state | status: :reconnecting}, []}

  def reduce(%__MODULE__{} = state, :submitting),
    do: {%{state | status: :submitting}, []}

  def reduce(%__MODULE__{} = state, {:launch, installation_id}) do
    case launch_entry(state, installation_id) do
      {:ok, entry} -> {state, [{:handoff, entry}]}
      {:error, :access_revoked} -> revoked_transition(state, "access_revoked")
      {:error, :unavailable} -> {state, [{:flash, :error, "That installed UI is unavailable."}]}
    end
  end

  def reduce(%__MODULE__{} = state, {:loaded, result}), do: {complete(state, result), []}

  def reduce(%__MODULE__{} = state, {:handoff_result, entry, {:ok, url}}) do
    {activate(state, entry), [{:launch, entry, url}]}
  end

  def reduce(%__MODULE__{} = state, {:handoff_result, _entry, {:error, :access_revoked}}),
    do: revoked_transition(state, "access_revoked")

  def reduce(%__MODULE__{} = state, {:handoff_result, _entry, {:error, _reason}}),
    do: {state, [{:flash, :error, "The installed UI could not be launched."}]}

  # Kept as a small compatibility boundary for callers delivering raw RPC results.
  def reduce(%__MODULE__{} = state, {:complete, result}), do: reduce(state, {:loaded, result})

  def reduce(%__MODULE__{} = state, _event), do: {state, []}

  @doc "Loads safe installed UI metadata through the generated RPC client."
  @spec execute(t(), {:load, HephaestusWeb.Identity.t(), non_neg_integer()}) ::
          {:loaded, {:ok, map()} | {:error, term()}}
  @spec execute(t(), {:handoff, HephaestusWeb.Identity.t(), map()}) ::
          {:handoff_result, map(), {:ok, String.t()} | {:error, atom()}}
  @spec execute(t(), {:handoff, HephaestusWeb.Identity.t(), map(), keyword()}) ::
          {:handoff_result, map(), {:ok, String.t()} | {:error, atom()}}
  def execute(%__MODULE__{} = state, {:load, identity, _generation}) do
    result =
      Client.list_ui_installations(
        identity,
        state.data.organization_id,
        state.data.target,
        state.data.request_page_token
      )

    {:loaded, result}
  end

  def execute(%__MODULE__{} = _state, {:handoff, identity, entry}) do
    {:handoff_result, entry, create_handoff_url(identity, entry)}
  end

  def execute(%__MODULE__{} = _state, {:handoff, identity, entry, options}) do
    {:handoff_result, entry, create_handoff_url(identity, entry, options)}
  end

  @doc "Completes a navigation load and removes terminal installations."
  @spec complete(t(), {:ok, map()} | {:error, term()}) :: t()
  def complete(%__MODULE__{} = state, {:ok, %{"installations" => installations} = response}) do
    token = state.data.request_page_token
    installations = Enum.reject(installations, &(&1["lifecycle"] == "removed"))
    previous_installations = Map.get(state.data.pages, token, [])
    previous_ids = MapSet.new(previous_installations, & &1["installation_id"])
    current_ids = MapSet.new(installations, & &1["installation_id"])
    removed_ids = MapSet.difference(previous_ids, current_ids)

    pages =
      state.data.pages
      |> Enum.map(fn {page_token, entries} ->
        {page_token, Enum.reject(entries, &MapSet.member?(removed_ids, &1["installation_id"]))}
      end)
      |> Map.new()
      |> Map.put(token, installations)

    page_order =
      if token in state.data.page_order,
        do: state.data.page_order,
        else: state.data.page_order ++ [token]

    page = Map.get(response, "page") || %{}
    page_next_token = page |> Map.get("next_page_token") |> blank_to_nil()
    revision = state.data.revision + 1
    page_versions = Map.put(state.data.page_versions, token, revision)
    page_next_tokens = Map.put(state.data.page_next_tokens, token, page_next_token)
    installations = merged_installations(pages, page_order, page_versions)
    next_page_token = next_page_token(page_order, page_next_tokens)

    state
    |> put_data(%{
      installations: installations,
      terminal: false,
      next_page_token: next_page_token,
      loading_more: false,
      pages: pages,
      page_order: page_order,
      page_next_tokens: page_next_tokens,
      page_versions: page_versions,
      refresh_cursor: token,
      revision: revision
    })
    |> Map.put(:status, :ready)
    |> Map.put(:error, nil)
    |> retain_active(installations)
  end

  def complete(%__MODULE__{} = state, {:error, %Error{kind: kind}})
      when kind in [:permission_denied, :not_found, :unauthenticated] do
    state
    |> put_data(%{
      installations: [],
      terminal: false,
      next_page_token: nil,
      request_page_token: "",
      loading_more: false,
      pages: %{},
      page_order: [],
      page_next_tokens: %{},
      page_versions: %{},
      refresh_cursor: nil,
      revision: 0,
      active_installation_id: nil,
      active_generation_id: nil
    })
    |> Map.put(:status, :access_revoked)
    |> Map.put(:error, "Installed UIs are no longer available.")
  end

  def complete(
        %__MODULE__{data: %{request_page_token: token}} = state,
        {:error, %Error{kind: kind}}
      )
      when token != "" and kind in [:invalid, :not_found, :precondition],
      do: reset_pagination(state)

  def complete(%__MODULE__{} = state, {:error, _reason}) do
    state
    |> put_data(%{loading_more: false})
    |> Map.put(:status, :error)
    |> Map.put(:error, "Installed UIs are temporarily unavailable.")
  end

  @doc "Returns the reviewed shell projection used by presentation components."
  @spec present(t()) :: map()
  def present(%__MODULE__{} = state) do
    %{
      state: presentation_status(state),
      installations: state.data.installations,
      error: state.error,
      has_more: is_binary(state.data.next_page_token),
      loading_more: state.data.loading_more
    }
  end

  @doc "Prepares a bounded request for the next page in the same target window."
  @spec load_more(t()) :: t()
  def load_more(%__MODULE__{data: %{next_page_token: token}} = state) when is_binary(token) do
    put_data(state, %{request_page_token: token, loading_more: true})
  end

  def load_more(%__MODULE__{} = state), do: state

  @doc "Refreshes one retained page, prioritizing the page containing the active UI."
  @spec refresh(t()) :: t()
  def refresh(%__MODULE__{} = state) do
    token = refresh_page_token(state)
    put_data(state, %{request_page_token: token, loading_more: false})
  end

  @doc "Clears only the active browser selection after an explicit user close."
  @spec deactivate(t()) :: t()
  def deactivate(%__MODULE__{} = state),
    do: put_data(state, %{active_installation_id: nil, active_generation_id: nil})

  @doc "Resolves one launch request against the current safe projection."
  @spec launch_entry(t(), String.t()) :: {:ok, map()} | {:error, :unavailable | :access_revoked}
  def launch_entry(%__MODULE__{status: :access_revoked}, _installation_id),
    do: {:error, :access_revoked}

  def launch_entry(%__MODULE__{data: %{terminal: true}}, _installation_id),
    do: {:error, :unavailable}

  def launch_entry(%__MODULE__{} = state, installation_id) when is_binary(installation_id) do
    case Enum.find(state.data.installations, &(&1["installation_id"] == installation_id)) do
      %{"lifecycle" => "enabled", "launchable" => true} = entry ->
        if valid_entry?(entry), do: {:ok, entry}, else: {:error, :unavailable}

      _entry ->
        {:error, :unavailable}
    end
  end

  def launch_entry(_state, _installation_id), do: {:error, :unavailable}

  defp create_handoff_url(identity, entry, options \\ []) do
    browser_options = Keyword.get(options, :ui_browser_options, [])

    invoke_options =
      options
      |> Keyword.delete(:ui_browser_options)
      |> Keyword.put(:on_success, fn handoff, secret ->
        projection = Map.put(handoff, "route_base", entry["route_base"])
        UIBrowser.bootstrap_url(projection, secret, "light", browser_options)
      end)

    result =
      Client.create_ui_browser_handoff(
        identity,
        entry["installation_id"],
        entry["generation_id"],
        entry["route_base"],
        invoke_options
      )

    case result do
      {:ok, url} ->
        {:ok, url}

      {:error, %Error{kind: kind}} when kind in [:permission_denied, :not_found] ->
        {:error, :access_revoked}

      {:error, _reason} ->
        {:error, :handoff_failed}
    end
  end

  @doc "Records the safe installation and generation currently shown in the browser."
  @spec activate(t(), map()) :: t()
  def activate(%__MODULE__{} = state, entry),
    do:
      state
      |> put_data(%{
        active_installation_id: entry["installation_id"],
        active_generation_id: entry["generation_id"]
      })
      |> retain_active(state.data.installations)

  @doc "Marks the shell unavailable after a terminal or revoked authority result."
  @spec terminate(t()) :: t()
  def terminate(%__MODULE__{} = state) do
    state
    |> put_data(%{
      installations: [],
      terminal: true,
      next_page_token: nil,
      request_page_token: "",
      loading_more: false,
      pages: %{},
      page_order: [],
      page_next_tokens: %{},
      page_versions: %{},
      refresh_cursor: nil,
      revision: 0,
      active_installation_id: nil,
      active_generation_id: nil
    })
    |> Map.put(:status, :access_revoked)
    |> Map.put(:error, "Installed UI access ended.")
  end

  defp retain_active(%__MODULE__{} = state, installations) do
    active? =
      is_binary(state.data.active_installation_id) and
        is_binary(state.data.active_generation_id) and
        Enum.any?(installations, fn entry ->
          entry["installation_id"] == state.data.active_installation_id and
            entry["generation_id"] == state.data.active_generation_id and
            entry["lifecycle"] == "enabled" and entry["launchable"] == true
        end)

    if active?,
      do: state,
      else: put_data(state, %{active_installation_id: nil, active_generation_id: nil})
  end

  defp merged_installations(pages, page_order, page_versions) do
    latest =
      Enum.reduce(page_order, %{}, fn token, entries ->
        version = Map.get(page_versions, token, 0)

        Enum.reduce(Map.get(pages, token, []), entries, fn entry, entries ->
          id = entry["installation_id"]

          case Map.get(entries, id) do
            nil ->
              Map.put(entries, id, {version, entry})

            {existing_version, _existing_entry} when version >= existing_version ->
              Map.put(entries, id, {version, entry})

            _existing ->
              entries
          end
        end)
      end)

    page_order
    |> Enum.flat_map(&Map.get(pages, &1, []))
    |> Enum.reduce({MapSet.new(), []}, fn entry, {seen, ordered} ->
      id = entry["installation_id"]

      if MapSet.member?(seen, id) do
        {seen, ordered}
      else
        {MapSet.put(seen, id), [elem(Map.fetch!(latest, id), 1) | ordered]}
      end
    end)
    |> elem(1)
    |> Enum.reverse()
  end

  defp next_page_token([], _page_next_tokens), do: nil

  defp next_page_token(page_order, page_next_tokens) do
    page_order
    |> List.last()
    |> then(&Map.get(page_next_tokens, &1))
  end

  defp refresh_page_token(%__MODULE__{data: %{page_order: []}}), do: ""

  defp refresh_page_token(%__MODULE__{} = state) do
    active_token =
      if is_binary(state.data.active_installation_id) do
        Enum.find(state.data.page_order, fn token ->
          Enum.any?(Map.get(state.data.pages, token, []), fn entry ->
            entry["installation_id"] == state.data.active_installation_id
          end)
        end)
      end

    active_token || rotate_refresh_page(state.data.page_order, state.data.refresh_cursor)
  end

  defp rotate_refresh_page(page_order, nil), do: List.first(page_order)

  defp rotate_refresh_page(page_order, cursor) do
    case Enum.find_index(page_order, &(&1 == cursor)) do
      nil -> List.first(page_order)
      index -> Enum.at(page_order, rem(index + 1, length(page_order)))
    end
  end

  defp reset_pagination(%__MODULE__{} = state) do
    state
    |> put_data(%{
      installations: [],
      terminal: false,
      next_page_token: nil,
      request_page_token: "",
      loading_more: false,
      pages: %{},
      page_order: [],
      page_next_tokens: %{},
      page_versions: %{},
      refresh_cursor: nil,
      revision: 0,
      active_installation_id: nil,
      active_generation_id: nil
    })
    |> Map.put(:status, :error)
    |> Map.put(:error, "Installed UIs are temporarily unavailable.")
  end

  defp put_data(%__MODULE__{} = state, updates),
    do: %{state | data: Map.merge(state.data, updates)}

  defp initial_data(organization_id, target) do
    %{
      organization_id: organization_id,
      target: target,
      installations: [],
      terminal: false,
      active_installation_id: nil,
      active_generation_id: nil,
      next_page_token: nil,
      request_page_token: "",
      loading_more: false,
      pages: %{},
      page_order: [],
      page_next_tokens: %{},
      page_versions: %{},
      refresh_cursor: nil,
      revision: 0
    }
  end

  defp presentation_status(%__MODULE__{data: %{terminal: true}}), do: :terminated
  defp presentation_status(%__MODULE__{status: :initial}), do: :loading

  defp presentation_status(%__MODULE__{status: status})
       when status in [:loading, :stale, :reconnecting],
       do: :loading

  defp presentation_status(%__MODULE__{status: :submitting}), do: :loading
  defp presentation_status(%__MODULE__{status: :ready}), do: :ready
  defp presentation_status(%__MODULE__{status: :access_revoked}), do: :access_revoked
  defp presentation_status(%__MODULE__{status: :error}), do: :error

  defp blank_to_nil(nil), do: nil
  defp blank_to_nil(""), do: nil
  defp blank_to_nil(token) when is_binary(token), do: token
  defp blank_to_nil(_token), do: nil

  defp valid_entry?(entry) do
    is_binary(entry["installation_id"]) and entry["installation_id"] != "" and
      is_binary(entry["generation_id"]) and entry["generation_id"] != "" and
      is_binary(entry["route_base"]) and entry["route_base"] != ""
  end

  defp revoked_transition(state, reason) do
    {terminate(state),
     [
       {:close, reason},
       {:flash, :error, "Installed UI access was revoked."}
     ]}
  end
end
