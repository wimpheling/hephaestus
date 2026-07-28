defmodule HephaestusWeb.RunNotifier do
  @moduledoc """
  Converts payload-free PostgreSQL wakeups into payload-free PubSub wakeups.

  Authorization is deliberately absent here: every LiveView must authorize
  before subscribing and must re-fetch protected data through RLS after each
  wakeup.
  """

  use GenServer

  @channel "hephaestus_ui_wakeup"

  def start_link(_options), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  def subscribe(run_id) do
    Phoenix.PubSub.subscribe(HephaestusWeb.PubSub, "run:#{run_id}")
  end

  def unsubscribe(run_id) do
    Phoenix.PubSub.unsubscribe(HephaestusWeb.PubSub, "run:#{run_id}")
  end

  def subscribe_repositories do
    Phoenix.PubSub.subscribe(HephaestusWeb.PubSub, "repository-wakeups")
  end

  @impl true
  def init(_state) do
    case Application.get_env(:hephaestus_web, :database_url) do
      nil ->
        {:ok, nil}

      _database_url ->
        {:ok, notifications} = Postgrex.Notifications.start_link(HephaestusWeb.Repo.config())
        {:ok, reference} = Postgrex.Notifications.listen(notifications, @channel)
        {:ok, {notifications, reference}}
    end
  end

  @impl true
  def handle_info({:notification, _pid, _reference, @channel, payload}, state) do
    with {:ok, %{"id" => run_id}} <- Jason.decode(payload) do
      Phoenix.PubSub.broadcast(
        HephaestusWeb.PubSub,
        "run:#{run_id}",
        {:run_wakeup, run_id}
      )

      Phoenix.PubSub.broadcast(
        HephaestusWeb.PubSub,
        "repository-wakeups",
        :repository_wakeup
      )
    end

    {:noreply, state}
  end
end
