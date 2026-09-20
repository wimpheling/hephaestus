defmodule HephaestusWebWeb.InstalledUiNavigationState do
  @moduledoc """
  Finite navigation state for installed UI cards in an authenticated shell.

  The state keeps only the safe projection returned by the release RPC. A
  handoff bearer and bootstrap URL are created only for the one launch event
  and never become LiveView assigns.
  """

  alias HephaestusWeb.RPC.{Client, Error}

  @type t :: %__MODULE__{
          organization_id: String.t(),
          target: :global | {:project, String.t()} | {:repository, String.t()},
          status: :loading | :ready | :error | :access_revoked | :terminated,
          installations: [map()],
          error: String.t() | nil,
          active_installation_id: String.t() | nil,
          active_generation_id: String.t() | nil,
          next_page_token: String.t() | nil,
          request_page_token: String.t(),
          loading_more: boolean(),
          pages: %{optional(String.t()) => [map()]},
          page_order: [String.t()],
          page_next_tokens: %{optional(String.t()) => String.t() | nil},
          page_versions: %{optional(String.t()) => pos_integer()},
          refresh_cursor: String.t() | nil,
          revision: non_neg_integer()
        }

  defstruct organization_id: nil,
            target: :global,
            status: :loading,
            installations: [],
            error: nil,
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

  @doc "Creates a loading state for one explicit organization and target."
  @spec new(String.t(), :global | {:project, String.t()} | {:repository, String.t()}) :: t()
  def new(organization_id, target),
    do: %__MODULE__{organization_id: organization_id, target: target}

  @doc "Loads safe installed UI metadata through the generated RPC client."
  @spec execute(t(), HephaestusWeb.Identity.t()) :: {:ok, map()} | {:error, term()}
  def execute(%__MODULE__{} = state, identity) do
    Client.list_ui_installations(
      identity,
      state.organization_id,
      state.target,
      state.request_page_token
    )
  end

  @doc "Completes a navigation load and removes terminal installations."
  @spec complete(t(), {:ok, map()} | {:error, term()}) :: t()
  def complete(%__MODULE__{} = state, {:ok, %{"installations" => installations} = response}) do
    token = state.request_page_token
    installations = Enum.reject(installations, &(&1["lifecycle"] == "removed"))
    previous_installations = Map.get(state.pages, token, [])
    previous_ids = MapSet.new(previous_installations, & &1["installation_id"])
    current_ids = MapSet.new(installations, & &1["installation_id"])

    removed_ids = MapSet.difference(previous_ids, current_ids)

    pages =
      state.pages
      |> Enum.map(fn {page_token, entries} ->
        {page_token, Enum.reject(entries, &MapSet.member?(removed_ids, &1["installation_id"]))}
      end)
      |> Map.new()
      |> Map.put(token, installations)

    page_order =
      if token in state.page_order, do: state.page_order, else: state.page_order ++ [token]

    page = Map.get(response, "page") || %{}
    page_next_token = page |> Map.get("next_page_token") |> blank_to_nil()
    revision = state.revision + 1
    page_versions = Map.put(state.page_versions, token, revision)
    page_next_tokens = Map.put(state.page_next_tokens, token, page_next_token)
    installations = merged_installations(pages, page_order, page_versions)
    next_page_token = next_page_token(page_order, page_next_tokens)

    %{
      state
      | status: :ready,
        installations: installations,
        error: nil,
        next_page_token: next_page_token,
        loading_more: false,
        pages: pages,
        page_order: page_order,
        page_next_tokens: page_next_tokens,
        page_versions: page_versions,
        refresh_cursor: token,
        revision: revision
    }
    |> retain_active(installations)
  end

  def complete(%__MODULE__{} = state, {:error, %Error{kind: kind}})
      when kind in [:permission_denied, :not_found, :unauthenticated] do
    %{
      state
      | status: :access_revoked,
        installations: [],
        error: "Installed UIs are no longer available.",
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

  def complete(
        %__MODULE__{request_page_token: token} = state,
        {:error, %Error{kind: kind}}
      )
      when token != "" and kind in [:invalid, :not_found, :precondition] do
    reset_pagination(state)
  end

  def complete(%__MODULE__{} = state, {:error, _reason}) do
    %{
      state
      | status: :error,
        error: "Installed UIs are temporarily unavailable.",
        loading_more: false
    }
  end

  @doc "Returns the reviewed shell projection used by presentation components."
  @spec present(t()) :: map()
  def present(%__MODULE__{} = state) do
    %{
      state: state.status,
      installations: state.installations,
      error: state.error,
      has_more: is_binary(state.next_page_token),
      loading_more: state.loading_more
    }
  end

  @doc "Prepares a bounded request for the next page in the same target window."
  @spec load_more(t()) :: t()
  def load_more(%__MODULE__{next_page_token: token} = state) when is_binary(token) do
    %{state | request_page_token: token, loading_more: true}
  end

  def load_more(%__MODULE__{} = state), do: state

  @doc "Refreshes one retained page, prioritizing the page containing the active UI."
  @spec refresh(t()) :: t()
  def refresh(%__MODULE__{} = state) do
    token = refresh_page_token(state)
    %{state | request_page_token: token, loading_more: false}
  end

  @doc "Clears only the active browser selection after an explicit user close."
  @spec deactivate(t()) :: t()
  def deactivate(%__MODULE__{} = state),
    do: %{state | active_installation_id: nil, active_generation_id: nil}

  @doc "Resolves one launch request against the current safe projection."
  @spec launch_entry(t(), String.t()) :: {:ok, map()} | {:error, :unavailable | :access_revoked}
  def launch_entry(%__MODULE__{status: :access_revoked}, _installation_id),
    do: {:error, :access_revoked}

  def launch_entry(%__MODULE__{} = state, installation_id) when is_binary(installation_id) do
    case Enum.find(state.installations, &(&1["installation_id"] == installation_id)) do
      %{"lifecycle" => "enabled", "launchable" => true} = entry ->
        if valid_entry?(entry), do: {:ok, entry}, else: {:error, :unavailable}

      _entry ->
        {:error, :unavailable}
    end
  end

  def launch_entry(_state, _installation_id), do: {:error, :unavailable}

  @doc "Records the safe installation and generation currently shown in the browser."
  @spec activate(t(), map()) :: t()
  def activate(%__MODULE__{} = state, entry),
    do:
      retain_active(
        %{
          state
          | active_installation_id: entry["installation_id"],
            active_generation_id: entry["generation_id"]
        },
        state.installations
      )

  @doc "Marks the shell unavailable after a terminal or revoked authority result."
  @spec terminate(t()) :: t()
  def terminate(%__MODULE__{} = state),
    do: %{
      state
      | status: :terminated,
        installations: [],
        error: "Installed UI access ended.",
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

  defp retain_active(%__MODULE__{} = state, installations) do
    active? =
      is_binary(state.active_installation_id) and
        is_binary(state.active_generation_id) and
        Enum.any?(installations, fn entry ->
          entry["installation_id"] == state.active_installation_id and
            entry["generation_id"] == state.active_generation_id and
            entry["lifecycle"] == "enabled" and entry["launchable"] == true
        end)

    if active?, do: state, else: %{state | active_installation_id: nil, active_generation_id: nil}
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

  defp refresh_page_token(%__MODULE__{page_order: []}), do: ""

  defp refresh_page_token(%__MODULE__{} = state) do
    active_token =
      if is_binary(state.active_installation_id) do
        Enum.find(state.page_order, fn token ->
          Enum.any?(Map.get(state.pages, token, []), fn entry ->
            entry["installation_id"] == state.active_installation_id
          end)
        end)
      end

    active_token || rotate_refresh_page(state.page_order, state.refresh_cursor)
  end

  defp rotate_refresh_page(page_order, nil), do: List.first(page_order)

  defp rotate_refresh_page(page_order, cursor) do
    case Enum.find_index(page_order, &(&1 == cursor)) do
      nil -> List.first(page_order)
      index -> Enum.at(page_order, rem(index + 1, length(page_order)))
    end
  end

  defp reset_pagination(%__MODULE__{} = state) do
    %{
      state
      | status: :error,
        installations: [],
        error: "Installed UIs are temporarily unavailable.",
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

  defp blank_to_nil(nil), do: nil
  defp blank_to_nil(""), do: nil
  defp blank_to_nil(token) when is_binary(token), do: token
  defp blank_to_nil(_token), do: nil

  defp valid_entry?(entry) do
    is_binary(entry["installation_id"]) and entry["installation_id"] != "" and
      is_binary(entry["generation_id"]) and entry["generation_id"] != "" and
      is_binary(entry["route_base"]) and entry["route_base"] != ""
  end
end
