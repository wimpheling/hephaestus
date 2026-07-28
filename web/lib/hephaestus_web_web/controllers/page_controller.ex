defmodule HephaestusWebWeb.PageController do
  use HephaestusWebWeb, :controller

  def home(conn, _params) do
    if conn.assigns.current_identity do
      redirect(conn, to: ~p"/organizations")
    else
      render(conn, :home)
    end
  end
end
