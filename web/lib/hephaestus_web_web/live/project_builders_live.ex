defmodule HephaestusWebWeb.ProjectBuildersLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.ProjectBuildersState

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    state = ProjectBuildersState.new(%{project_id: project_id})
    {state, _effects} = ProjectBuildersState.reduce(state, :load)
    socket = assign(socket, page_state: state, page_title: "Project builders")

    if connected?(socket) do
      {:ok, start_async(socket, :load, fn -> ProjectBuildersState.execute(state, {:load, socket.assigns.current_identity}) end)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_async(:load, {:ok, event}, socket), do: {:noreply, reduce(socket, event)}
  def handle_async(:load, {:exit, reason}, socket), do: {:noreply, reduce(socket, {:failed, reason})}
  def handle_async(:command, {:ok, event}, socket), do: {:noreply, reduce(socket, event)}
  def handle_async(:command, {:exit, reason}, socket), do: {:noreply, reduce(socket, {:failed, reason})}

  @impl true
  def handle_event("create-builder", %{"builder" => attributes}, socket) do
    state = socket.assigns.page_state
    {state, _effects} = ProjectBuildersState.reduce(state, :submitting)
    identity = socket.assigns.current_identity
    {:noreply, socket |> assign(:page_state, state) |> start_async(:command, fn -> ProjectBuildersState.execute(state, {:create, identity, attributes}) end)}
  end

  def handle_event("prepare-builder", %{"builder_id" => builder_id}, socket) do
    state = socket.assigns.page_state
    {state, _effects} = ProjectBuildersState.reduce(state, :submitting)
    identity = socket.assigns.current_identity
    {:noreply, socket |> assign(:page_state, state) |> start_async(:command, fn -> ProjectBuildersState.execute(state, {:prepare, identity, builder_id}) end)}
  end

  @impl true
  def render(assigns) do
    presentation = ProjectBuildersState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity} organizations_destination="/organizations" logout_destination="/logout">
      <main id="project-builders-page" class="summary-body">
        <nav class="breadcrumbs" aria-label="Breadcrumb"><a href={~p"/organizations"}>Organizations</a> / Project builders</nav>
        <h1>Project-owned builders</h1>
        <p>Define a Dockerfile-backed OCI builder from an approved platform image. Preparation is isolated and records an immutable digest.</p>
        <p :if={@presentation.error} role="alert">{@presentation.error}</p>
        <form id="create-project-builder" phx-submit="create-builder">
          <input name="builder[key]" placeholder="builder key" required />
          <input name="builder[display_name]" placeholder="display name" required />
          <input name="builder[source_repository_id]" placeholder="source repository UUID" required />
          <input name="builder[source_revision]" placeholder="source commit SHA" required />
          <input name="builder[dockerfile_path]" value="Dockerfile" required />
          <input name="builder[context_path]" value="." required />
          <input name="builder[context_digest]" placeholder="sha256:..." required />
          <input name="builder[approved_base_image_reference]" placeholder="approved base@sha256:..." required />
          <button type="submit">Create builder</button>
        </form>
        <section id="project-builder-list">
          <p :if={@presentation.builders == []}>No project builders defined.</p>
          <article :for={builder <- @presentation.builders} id={"project-builder-#{builder["id"]}">
            <h2>{builder["display_name"]} <code>{builder["key"]}</code></h2>
            <p>Status: {builder["status"]}</p>
            <p>Dockerfile: <code>{builder["dockerfile_path"]}</code></p>
            <p :if={builder["oci_image_digest"]}>Prepared digest: <code>{builder["oci_image_digest"]}</code></p>
            <form :if={builder["status"] in ["draft", "failed"]} phx-submit="prepare-builder">
              <input type="hidden" name="builder_id" value={builder["id"]} />
              <button type="submit">Prepare image</button>
            </form>
          </article>
        </section>
      </main>
    </Layouts.app>
    """
  end

  defp reduce(socket, event) do
    {state, _effects} = ProjectBuildersState.reduce(socket.assigns.page_state, event)
    assign(socket, :page_state, state)
  end
end
