defmodule HephaestusWebWeb.DesignSystem.Pages.PersonalAccessTokensPage do
  @moduledoc "Pure presentation for developer Git personal access tokens."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :tokens, :list, default: []
  attr :item_count, :integer, default: 0
  attr :form, :map, required: true
  attr :error, :any, default: nil
  attr :create_event, :string, required: true
  attr :rotate_event, :string, required: true
  attr :revoke_event, :string, required: true

  @doc "Renders safe PAT metadata and bounded lifecycle controls."
  def personal_access_tokens_page(assigns) do
    ~H"""
    <.page_state
      id="personal-access-tokens-page-state"
      state={@state}
      title="Git credentials unavailable"
      message={@error || "Developer Git credentials are not ready."}
    >
      <.frame variant={:summary_body}>
        <.page_heading
          eyebrow="Developer authentication"
          title="Git credentials"
          description="Create narrowly scoped personal access tokens for Git over HTTPS. Browser sign-in continues to use OIDC."
        >
          <:actions>
            <.tag>{@item_count} credentials</.tag>
          </:actions>
        </.page_heading>

        <.frame as="section" variant={:panel}>
          <.text as="h2" variant={:subtitle}>Create credential</.text>
          <.text as="p" variant={:muted}>
            The bearer value opens in a one-time copy dialog and is never stored by Hephaestus.
          </.text>
          <.form_container
            for={to_form(@form, as: :token)}
            id="create-personal-access-token"
            submit={@create_event}
          >
            <.input name="token[label]" value={@form["label"]} label="Label" required />
            <.input
              name="token[operations][]"
              type="select"
              value={@form["operations"]}
              label="Git operations"
              options={operation_options()}
              multiple
              required
            />
            <.input
              name="token[repository_ids]"
              value={@form["repository_ids"]}
              label="Exact repository IDs (comma separated, optional)"
            />
            <.input
              name="token[expires_at]"
              value={@form["expires_at"]}
              type="datetime-local"
              label="Expires at (UTC, maximum 90 days)"
              required
            />
            <.action interaction={:submit} variant={:primary}>Create credential</.action>
          </.form_container>
        </.frame>

        <.text :if={@tokens == []} as="p" id="personal-access-tokens-empty" variant={:empty}>
          No developer Git credentials have been created.
        </.text>

        <.frame
          :for={token <- @tokens}
          as="article"
          id={"personal-access-token-#{token["id"]}"}
          variant={:proposal}
        >
          <.frame variant={:summary_body}>
            <.text as="strong">{token["label"]}</.text>
            <.tag>{status(token)}</.tag>
            <.text as="span" variant={:mono}>{token["id"]}</.text>
            <.text as="span" variant={:muted}>
              {Enum.join(get_in(token, ["scope", "operations"]) || [], ", ")}
            </.text>
            <.text as="span" variant={:muted}>Expires {format_time(token["expires_at"])}</.text>
            <.text as="span" variant={:muted}>
              Last used {format_time(token["last_used_at"])}
            </.text>
          </.frame>

          <.frame :if={is_nil(token["revoked_at"])} variant={:resource_controls}>
            <.form_container
              for={to_form(rotation_form(token, @form), as: :rotation)}
              id={"rotate-personal-access-token-#{token["id"]}"}
              submit={@rotate_event}
              layout={:inline}
            >
              <.input name="token_id" type="hidden" value={token["id"]} />
              <.input
                name="rotation[label]"
                value={token["label"]}
                aria_label="Replacement label"
                required
              />
              <.input
                name="rotation[operations][]"
                type="select"
                value={get_in(token, ["scope", "operations"]) || []}
                aria_label="Replacement Git operations"
                options={operation_options()}
                multiple
                required
              />
              <.input
                name="rotation[repository_ids]"
                value={repository_ids(token)}
                aria_label="Replacement repository IDs"
              />
              <.input
                name="rotation[expires_at]"
                value={@form["expires_at"]}
                type="datetime-local"
                aria_label="Replacement expiry"
                required
              />
              <.action interaction={:submit} variant={:compact}>Rotate</.action>
            </.form_container>
            <.action
              interaction={:event}
              event={@revoke_event}
              event_payload={%{token_id: token["id"]}}
              variant={:danger_compact}
            >
              Revoke
            </.action>
          </.frame>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp operation_options,
    do: [{"Discover refs", "discover"}, {"Fetch", "fetch"}, {"Push", "receive"}]

  defp rotation_form(token, defaults),
    do: %{
      "label" => token["label"],
      "operations" => get_in(token, ["scope", "operations"]) || [],
      "repository_ids" => repository_ids(token),
      "expires_at" => defaults["expires_at"]
    }

  defp repository_ids(token),
    do: token |> get_in(["scope", "repository_ids"]) |> List.wrap() |> Enum.join(",")

  defp status(%{"revoked_at" => %DateTime{}}), do: "revoked"

  defp status(%{"expires_at" => %DateTime{} = expiry}) do
    if DateTime.compare(expiry, DateTime.utc_now()) in [:lt, :eq], do: "expired", else: "active"
  end

  defp status(_token), do: "unknown"

  defp format_time(nil), do: "never"
  defp format_time(%DateTime{} = value), do: Calendar.strftime(value, "%Y-%m-%d %H:%M UTC")
  defp format_time(_value), do: "unknown"
end
