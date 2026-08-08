defmodule HephaestusWebWeb.PersonalAccessTokensState do
  @moduledoc "State and effects for developer Git personal access token management."

  alias HephaestusWeb.RPC.Client

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

  defstruct status: :initial,
            data: %{tokens: []},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def statuses, do: @statuses

  def new(_params) do
    %__MODULE__{form: default_form()}
  end

  def reduce(state, :load) do
    generation = state.stream_generation + 1

    {%{state | status: :loading, error: nil, stream_generation: generation},
     [{:load, generation}]}
  end

  def reduce(state, {:loaded, generation, tokens}) when generation == state.stream_generation,
    do: {%{state | status: :ready, data: %{tokens: tokens}, error: nil}, []}

  def reduce(state, {:loaded, _generation, _tokens}), do: {state, []}

  def reduce(state, :submitting),
    do: {%{state | status: :submitting, error: nil}, []}

  def reduce(state, {:issued, value}) when is_binary(value) do
    generation = state.stream_generation + 1

    {%{state | status: :submitting, form: default_form(), stream_generation: generation},
     [
       {:reveal, value},
       {:flash, :info, "Credential issued. Copy it now; it will not be shown again."},
       {:load, generation}
     ]}
  end

  def reduce(state, :revoked) do
    generation = state.stream_generation + 1

    {%{state | status: :submitting, stream_generation: generation},
     [{:flash, :info, "Credential revoked."}, {:load, generation}]}
  end

  def reduce(state, {:failed, _reason}),
    do:
      {%{state | status: :error, error: "Credential request was denied or failed validation."},
       []}

  def reduce(state, :stale), do: reduce(%{state | status: :stale}, :load)
  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}

  def present(state) do
    %{
      status: presentation_status(state.status),
      tokens: state.data.tokens,
      item_count: length(state.data.tokens),
      form: state.form,
      error: state.error
    }
  end

  def execute(_state, {:load, identity, generation}) do
    case Client.list_personal_access_tokens(identity) do
      {:ok, tokens} -> {:loaded, generation, tokens}
      {:error, reason} -> {:failed, reason}
    end
  end

  def execute(_state, {:create, identity, attributes}) do
    identity
    |> Client.create_personal_access_token(attributes)
    |> issued_result()
  end

  def execute(_state, {:rotate, identity, token_id, attributes}) do
    identity
    |> Client.rotate_personal_access_token(token_id, attributes)
    |> issued_result()
  end

  def execute(_state, {:revoke, identity, token_id}) do
    case Client.revoke_personal_access_token(identity, token_id) do
      {:ok, _response} -> :revoked
      {:error, reason} -> {:failed, reason}
    end
  end

  defp issued_result({:ok, response}) do
    case get_in(response, ["value", "value"]) do
      value when is_binary(value) and byte_size(value) > 0 -> {:issued, value}
      _missing -> {:failed, :missing_one_time_value}
    end
  end

  defp issued_result({:error, reason}), do: {:failed, reason}

  defp default_form do
    %{
      "label" => "",
      "operations" => ["discover", "fetch"],
      "repository_ids" => "",
      "expires_at" =>
        DateTime.utc_now()
        |> DateTime.add(30, :day)
        |> Calendar.strftime("%Y-%m-%dT%H:%M")
    }
  end

  defp presentation_status(status) when status in [:ready, :submitting], do: :ready
  defp presentation_status(status) when status in [:initial, :loading], do: :loading
  defp presentation_status(status) when status in [:stale, :reconnecting], do: :reconnecting
  defp presentation_status(_status), do: :error
end
