defmodule HephaestusWebWeb.Router do
  use HephaestusWebWeb, :router

  pipeline :browser do
    plug :accepts, ["html"]
    plug :fetch_session
    plug :fetch_live_flash
    plug :put_root_layout, html: {HephaestusWebWeb.DesignSystem, :root}
    plug :protect_from_forgery
    plug :put_secure_browser_headers
    plug HephaestusWebWeb.UserAuth, :fetch_current_identity
  end

  pipeline :authenticated do
    plug HephaestusWebWeb.UserAuth, :require_authenticated
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
      live "/organizations/:organization_id", OrganizationWorkspaceLive, :projects
      live "/builders", BuilderCatalogLive, :index
      live "/organizations/:organization_id/projects/new", ProjectNewLive, :new_project
      live "/organizations/:organization_id/secrets", OrganizationSecretsLive, :secrets

      live "/organizations/:organization_id/secrets/new",
           OrganizationNewSecretLive,
           :new_secret

      live "/organizations/:organization_id/secret-grants/new",
           OrganizationNewGrantLive,
           :new_grant

      live "/projects/:project_id", ProjectLive, :repositories
      live "/projects/:project_id/repositories", ProjectLive, :repositories
      live "/projects/:project_id/repositories/new", RepositoryNewLive, :new_repository
      live "/projects/:project_id/agents", ProjectAgentsLive, :agents
      live "/projects/:project_id/agents/:instance_id", AgentInstanceLive, :show
      live "/projects/:project_id/runs", ProjectRunsLive, :runs
      live "/projects/:project_id/settings", ProjectSettingsLive, :settings
      live "/repositories/:repository_id", RepositoryFilesLive, :files
      live "/repositories/:repository_id/files", RepositoryFilesLive, :files
      live "/repositories/:repository_id/files/*path", RepositoryFilesLive, :files
      live "/repositories/:repository_id/commits", RepositoryCommitsLive, :commits
      live "/repositories/:repository_id/branches", RepositoryBranchesLive, :branches
      live "/repositories/:repository_id/builds", RepositoryBuildsLive, :builds
      live "/repositories/:repository_id/builds/:build_id", BuildLive, :show
      live "/repositories/:repository_id/releases", RepositoryReleasesLive, :releases
      live "/repositories/:repository_id/releases/:release_id", ReleaseLive, :show
      live "/repositories/:repository_id/agents", RepositoryAgentsLive, :agents
      live "/runs/:run_id", RunLive
    end
  end

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
