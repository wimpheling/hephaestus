defmodule HephaestusWebWeb.PageStreamTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.RPC.Error
  alias HephaestusWebWeb.{PageStream, PageStreamProbeState, RunState}

  test "replacement terminates the old watch and tags the new exact-resume generation" do
    state = %PageStreamProbeState{data: %{test_owner: self()}}

    socket = %Phoenix.LiveView.Socket{
      assigns: %{__changed__: %{}, current_identity: :identity, page_state: state}
    }

    first = PageStream.start_watch(socket, PageStreamProbeState)
    first_pid = first.assigns.watch_task
    assert_receive {:probe_watch_started, ^first_pid, 1, nil}

    first_ref = Process.monitor(first_pid)

    resumed_state = %{
      first.assigns.page_state
      | cursor: %{committed: "cursor-7", seen_event_ids: [], versions: %{}}
    }

    replaced =
      first
      |> Phoenix.Component.assign(:page_state, resumed_state)
      |> PageStream.start_watch(PageStreamProbeState)

    assert_receive {:DOWN, ^first_ref, :process, ^first_pid, _reason}

    replacement_pid = replaced.assigns.watch_task
    assert_receive {:probe_watch_started, ^replacement_pid, 2, "cursor-7"}

    replacement_ref = Process.monitor(replacement_pid)
    PageStream.cancel(replacement_pid)
    assert_receive {:DOWN, ^replacement_ref, :process, ^replacement_pid, _reason}
  end

  test "a denied exact-scope watch invokes the page's reviewed revocation behavior" do
    state = RunState.new("missing-run") |> RunState.begin_watch()

    socket = %Phoenix.LiveView.Socket{
      assigns: %{__changed__: %{}, page_state: state, watch_task: self()}
    }

    {updated, effects} =
      PageStream.reduce_ended(socket, RunState, {:error, Error.local(:not_found)})

    assert updated.assigns.page_state.status == :access_revoked
    assert updated.assigns.watch_task == nil

    assert effects == [
             {:flash, :error, "Run not found or access was revoked."},
             {:navigate, "/organizations"}
           ]
  end
end
