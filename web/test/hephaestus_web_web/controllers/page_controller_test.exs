defmodule HephaestusWebWeb.PageControllerTest do
  use HephaestusWebWeb.ConnCase

  test "GET /", %{conn: conn} do
    conn = get(conn, ~p"/")
    response = html_response(conn, 200)
    assert response =~ "Code enters."
    assert response =~ "Sign in with OIDC"
  end
end
