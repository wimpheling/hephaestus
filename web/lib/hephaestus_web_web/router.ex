defmodule HephaestusWebWeb.Router do
  use HephaestusWebWeb, :router

  pipeline :browser do
    plug :accepts, ["html"]
    plug :fetch_session
    plug :fetch_live_flash
    plug :put_root_layout, html: {HephaestusWebWeb.Layouts, :root}
    plug :protect_from_forgery
    plug :put_secure_browser_headers
    plug HephaestusWebWeb.UserAuth, :fetch_current_identity
  end

  pipeline :authenticated do
    plug HephaestusWebWeb.UserAuth, :require_authenticated
  end

  pipeline :api do
    plug :accepts, ["json"]
  end

  scope "/", HephaestusWebWeb do
    pipe_through :browser

    get "/", PageController, :home
    get "/login", AuthController, :login
    get "/auth/oidc/callback", AuthController, :callback
    delete "/logout", AuthController, :logout
  end

  scope "/", HephaestusWebWeb do
    pipe_through [:browser, :authenticated]

    live_session :authenticated,
      on_mount: [{HephaestusWebWeb.UserAuth, :require_authenticated}] do
      live "/organizations", OrganizationLive
      live "/organizations/:organization_id", RepositoryIndexLive
      live "/repositories/:repository_id", RepositoryLive
      live "/runs/:run_id", RunLive
    end
  end

  # Other scopes may use custom stacks.
  # scope "/api", HephaestusWebWeb do
  #   pipe_through :api
  # end

  # Enable LiveDashboard in development
  if Application.compile_env(:hephaestus_web, :dev_routes) do
    # If you want to use the LiveDashboard in production, you should put
    # it behind authentication and allow only admins to access it.
    # If your application does not have an admins-only section yet,
    # you can use Plug.BasicAuth to set up some basic authentication
    # as long as you are also using SSL (which you should anyway).
    import Phoenix.LiveDashboard.Router

    scope "/dev" do
      pipe_through :browser

      live_dashboard "/dashboard", metrics: HephaestusWebWeb.Telemetry
    end
  end
end
