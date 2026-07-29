defmodule HephaestusWebWeb.ResourceComponents do
  @moduledoc """
  Presentational primitives for consistently structured resource lists.

  Callers own navigation, identifiers, labels, actions, and all domain data.
  This module owns only the shared list frame and column layout.
  """

  use Phoenix.Component

  attr :id, :string, required: true
  attr :columns, :string, required: true
  attr :update, :string, default: nil, values: [nil, "stream"]
  attr :class, :any, default: nil
  attr :rest, :global

  slot :header, required: true
  slot :row
  slot :empty

  def resource_list(assigns) do
    ~H"""
    <section
      id={@id}
      class={["resource-list", @class]}
      style={"--resource-list-columns: #{@columns}"}
      phx-update={@update}
      {@rest}
    >
      <div id={"#{@id}-heading"} class="resource-list-heading">
        {render_slot(@header)}
      </div>
      <div :if={@empty != []} id={"#{@id}-empty"} class="resource-list-empty">
        {render_slot(@empty)}
      </div>
      {render_slot(@row)}
    </section>
    """
  end
end
