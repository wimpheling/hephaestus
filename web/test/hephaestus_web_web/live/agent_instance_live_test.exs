defmodule HephaestusWebWeb.AgentInstanceLiveTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{AgentInstanceLive, AgentInstanceState}

  test "watch events refresh the rendered update projection" do
    assert {:ok, mounted} =
             AgentInstanceLive.mount(%{"instance_id" => "instance-1"}, %{}, socket())

    {loading, [{:load, 1, "instance-1"}]} =
      AgentInstanceState.reduce(mounted.assigns.page_state, :load)

    {ready, []} =
      AgentInstanceState.reduce(loading, {:loaded, 1, {:ok, instance([])}})

    socket =
      mounted
      |> Phoenix.Component.assign(:page_state, ready)
      |> Phoenix.Component.assign(:stream_generation, ready.stream_generation)

    response = %{
      cursor: "cursor-2",
      item: {:event, event(:agent_instance_changed, "instance-1")}
    }

    assert {:noreply, refreshing} =
             AgentInstanceLive.handle_info({:page_watch, 1, response}, socket)

    assert refreshing.assigns.page_state.status == :stale

    task = refreshing.assigns.snapshot_task
    assert %Task{} = task
    Task.shutdown(task, :brutal_kill)

    ref = make_ref()

    snapshot_task = %Task{
      owner: self(),
      pid: self(),
      ref: ref,
      mfa: {__MODULE__, :snapshot, 0}
    }

    refreshing = Phoenix.Component.assign(refreshing, :snapshot_task, snapshot_task)

    assert {:noreply, refreshed} =
             AgentInstanceLive.handle_info(
               {ref, {:loaded, 1, {:ok, instance([update("compatibility_unknown")])}}},
               refreshing
             )

    assert refreshed.assigns.page_state.status == :ready

    assert [%{"id" => "update-1", "state" => "compatibility_unknown"}] =
             refreshed.assigns.presentation.updates
  end

  test "stale watch generations cannot replace the current page state" do
    assert {:ok, mounted} =
             AgentInstanceLive.mount(%{"instance_id" => "instance-1"}, %{}, socket())

    state = AgentInstanceState.begin_watch(mounted.assigns.page_state)

    socket =
      mounted
      |> Phoenix.Component.assign(:page_state, state)
      |> Phoenix.Component.assign(:stream_generation, state.stream_generation)

    assert {:noreply, unchanged} =
             AgentInstanceLive.handle_info({:page_watch, 0, :stale}, socket)

    assert unchanged.assigns.page_state == state
    assert unchanged.assigns.presentation == socket.assigns.presentation
  end

  defp event(variant, aggregate_id) do
    %{
      id: "event-1",
      cursor: "cursor-2",
      aggregate_type: "agent_instance",
      aggregate_id: aggregate_id,
      aggregate_version: 1,
      payload: {variant, %{}}
    }
  end

  defp instance(updates) do
    %{
      "id" => "instance-1",
      "organization_id" => "org-1",
      "project_id" => "project-1",
      "active_revision_id" => "revision-1",
      "revisions" => [],
      "attachments" => [],
      "updates" => updates
    }
  end

  defp update(state), do: %{"id" => "update-1", "state" => state}

  defp socket do
    %Phoenix.LiveView.Socket{
      assigns: %{__changed__: %{}, current_identity: %{subject: "identity-1"}},
      private: %{live_temp: %{}, lifecycle: %Phoenix.LiveView.Lifecycle{}}
    }
  end
end
