defmodule HephaestusWebWeb.ReleaseLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWeb.Store

  @impl true
  def mount(%{"release_id" => release_id}, _session, socket) do
    case Store.get_release(socket.assigns.current_identity, release_id) do
      {:ok, release} ->
        {:ok,
         socket
         |> stream_configure(:artifacts, dom_id: &"release-artifact-#{&1["id"]}")
         |> stream_configure(:agents, dom_id: &"release-agent-#{&1["id"]}")
         |> assign(:page_title, "#{release["version"]} · Release")
         |> assign(:release, release)
         |> stream(:artifacts, release["artifacts"])
         |> stream(:agents, release["agents"])}

      {:error, _reason} ->
        {:ok,
         socket
         |> put_flash(:error, "That release is not visible.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <.breadcrumbs id="release-breadcrumbs">
        <:item navigate={~p"/organizations"}>Organizations</:item>
        <:item navigate={~p"/organizations/#{@release["organization_id"]}"}>
          {@release["organization_name"]}
        </:item>
        <:item navigate={~p"/projects/#{@release["project_id"]}"}>
          {@release["project_name"]}
        </:item>
        <:item navigate={~p"/repositories/#{@release["repository_id"]}/releases"}>
          {@release["repository_name"]}
        </:item>
        <:current>{@release["version"]}</:current>
      </.breadcrumbs>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Immutable reusable release</p>
          <h1>{@release["version"]}</h1>
          <p class="lede">
            Exact source, build, configuration, and artifact-manifest provenance.
          </p>
        </div>
        <.tag tone={state_tone(@release["state"])}>{@release["state"]}</.tag>
      </section>

      <section id="release-provenance" class="repository-table">
        <article class="repo-row">
          <span class="repo-name"><i class="repo-icon">C</i><span><strong>Source commit</strong><small>{@release[
            "source_ref"
          ]}</small></span></span>
          <.link
            navigate={
              ~p"/repositories/#{@release["repository_id"]}/commits?#{[ref: @release["source_ref"]]}"
            }
            aria-label={"Browse source commit #{@release["source_commit"]}"}
          >
            <code>{short_hash(@release["source_commit"])}</code>
          </.link>
          <span>Build {short_id(@release["build_request_id"])}</span>
          <span><.tag tone={state_tone(@release["build_state"])}>{@release["build_state"]}</.tag></span>
        </article>
        <article class="repo-row">
          <span class="repo-name"><i class="repo-icon">M</i><span><strong>Manifest</strong><small>SHA-256</small></span></span>
          <code>{short_hash(@release["manifest_hash"])}</code>
          <span>Configuration</span>
          <code>{short_hash(@release["configuration_hash"])}</code>
        </article>
      </section>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Immutable release contents</p>
          <h2>Imported runtime files</h2>
          <p class="lede">
            Files copied from the sealed build output and stored immutably.
          </p>
        </div>
      </section>
      <div id="release-artifacts" class="repository-table" phx-update="stream">
        <p class="hidden only:block empty-copy">No artifacts are visible.</p>
        <article :for={{dom_id, artifact} <- @streams.artifacts} id={dom_id} class="repo-row">
          <span class="repo-name">
            <i class="repo-icon">F</i>
            <span><strong>{artifact["path"]}</strong><small>{artifact["media_type"]}</small></span>
          </span>
          <span>{artifact["kind"]}</span>
          <code>0{Integer.to_string(artifact["mode"], 8)}</code>
          <span>{format_bytes(artifact["size_bytes"])} · {short_hash(artifact["content_hash"])}</span>
        </article>
      </div>

      <section class="section-heading spacious">
        <div>
          <p class="eyebrow">Project-ready exports</p>
          <h2>Runnable agent definitions</h2>
          <p class="lede">
            Validated runtime definitions that projects can import as agent instances.
          </p>
        </div>
      </section>
      <div id="release-agents" class="repository-table" phx-update="stream">
        <p class="hidden only:block empty-copy">No exported agents are visible.</p>
        <article :for={{dom_id, agent} <- @streams.agents} id={dom_id} class="repo-row">
          <span class="repo-name">
            <i class="repo-icon">A</i>
            <span><strong>{agent["display_name"]}</strong><small>{agent["agent_key"]}</small></span>
          </span>
          <span>{if(agent["requires_state"], do: "persistent state", else: "stateless")}</span>
          <span>{length(agent["parameter_schema"])} parameters</span>
          <span>{length(agent["secret_slot_schema"])} secret slots</span>
        </article>
      </div>
    </Layouts.app>
    """
  end

  defp short_id(nil), do: "—"
  defp short_id(value), do: String.slice(value, 0, 8)
  defp short_hash(nil), do: "—"

  defp short_hash(value) when is_binary(value) and byte_size(value) in [20, 32],
    do: value |> Base.encode16(case: :lower) |> String.slice(0, 16)

  defp short_hash(value) when is_binary(value), do: String.slice(value, 0, 16)

  defp format_bytes(bytes) when bytes < 1_024, do: "#{bytes} B"
  defp format_bytes(bytes), do: "#{Float.round(bytes / 1_024, 1)} KiB"

  defp state_tone(value) when value in ["published", "drafted", "succeeded"], do: "success"
  defp state_tone(value) when value in ["revoked", "failed"], do: "danger"
  defp state_tone(_value), do: "neutral"
end
