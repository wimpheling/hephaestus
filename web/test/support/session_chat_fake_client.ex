defmodule HephaestusWebWeb.SessionChatNewStateTest.FakeClient do
  alias Hephaestus.Common.V1.OpaqueId

  alias Hephaestus.Release.V1.{
    ReleaseUiDescriptor,
    ReleaseUiPresentation,
    ReleaseUiRepositoryGitAccess,
    ReleaseUiScope,
    ReleaseUiStaticContent,
    ReleaseUiStaticFile
  }

  alias HephaestusWeb.RPC.Projection

  def get_release(_identity, _release_id),
    do: {:ok, %{"ui_descriptors" => [reference_ui_descriptor()]}}

  defp reference_ui_descriptor do
    Projection.to_value(%ReleaseUiDescriptor{
      key: "session-chat",
      scope: ReleaseUiScope.RELEASE_UI_SCOPE_REPOSITORY,
      presentation: ReleaseUiPresentation.RELEASE_UI_PRESENTATION_FULL_PAGE,
      route_base: "session-chat",
      repository_git_access:
        ReleaseUiRepositoryGitAccess.RELEASE_UI_REPOSITORY_GIT_ACCESS_READ_WRITE,
      content:
        {:static_content,
         %ReleaseUiStaticContent{
           files: [
             %ReleaseUiStaticFile{
               route: "index.html",
               artifact_id: %OpaqueId{value: "artifact-1"},
               media_type: "text/html"
             }
           ]
         }}
    })
  end

  def create_repository(_identity, _project_id, _name, _branch, _public, _runs, _options) do
    bump(:create_repository)
    {:ok, %{"repository_id" => "repo-1"}}
  end

  def import_agent(_identity, _project_id, release_agent_id, name, parameters, _policy, _options) do
    bump(:import_agent)
    Process.put(:session_chat_import_release_agent_id, release_agent_id)
    Process.put(:session_chat_import_name, name)
    Process.put(:session_chat_import_parameters, parameters)
    {:ok, %{"instance_id" => "instance-1", "revision_id" => "revision-1"}}
  end

  def revise_capabilities(_identity, _instance_id, _revision_id, _bindings, _options) do
    bump(:revise_capabilities)

    if Process.get(:session_chat_fake_revised, false) do
      {:ok, %{"instance_revision_id" => "revision-2"}}
    else
      Process.put(:session_chat_fake_revised, true)
      {:error, :capability_unavailable}
    end
  end

  def create_attachment(_identity, _instance_id, _repository_id, _selector, _trigger, _options) do
    bump(:create_attachment)
    {:ok, %{"attachment_id" => "attachment-1"}}
  end

  def bind_secret(_identity, _attributes, _options) do
    bump(:bind_secret)
    {:ok, %{"binding_id" => "binding-1"}}
  end

  def declare_brokered_https_rule(_identity, attributes, options) do
    bump(:declare_brokered_https_rule)
    Process.put(:session_chat_rule_declaration, {attributes, options})

    rule_id =
      if Process.get(:session_chat_fake_wrong_rule, false),
        do: "00000000-0000-4000-8000-000000000099",
        else: attributes["requested_rule_id"]

    {:ok, %{"rule_id" => rule_id}}
  end

  def install_ui(_identity, _organization_id, _target, _release_id, _ui_key, _options) do
    bump(:install_ui)

    if Process.get(:session_chat_fail_install_once, false) do
      Process.delete(:session_chat_fail_install_once)
      {:error, :install_failed}
    else
      {:ok, %{"installation_id" => "installation-1", "generation_id" => "generation-1"}}
    end
  end

  def create_ui_browser_handoff(_identity, _installation_id, _generation_id, _route, _options) do
    bump(:handoff)
    {:ok, "https://ui.test/session-chat"}
  end

  defp bump(key) do
    calls = Process.get(:session_chat_fake_calls, %{})
    Process.put(:session_chat_fake_calls, Map.update(calls, key, 1, &(&1 + 1)))
  end
end
