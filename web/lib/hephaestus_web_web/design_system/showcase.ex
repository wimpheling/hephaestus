defmodule HephaestusWebWeb.DesignSystem.Showcase do
  @moduledoc "Rendering examples for every public component and composite."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @doc "Returns the literal showcase registry consumed by architecture parity checks."
  def examples do
    [
      %{id: :action, render: :action_example},
      %{id: :app, render: :app_example},
      %{id: :breadcrumbs, render: :breadcrumbs_example},
      %{id: :button, render: :button_example},
      %{id: :flash, render: :flash_example},
      %{id: :flash_group, render: :flash_group_example},
      %{id: :form_container, render: :form_container_example},
      %{id: :frame, render: :frame_example},
      %{id: :glyph, render: :glyph_example},
      %{id: :header, render: :header_example},
      %{id: :icon, render: :icon_example},
      %{id: :input, render: :input_example},
      %{id: :list, render: :list_example},
      %{id: :repository_tree, render: :repository_tree_example},
      %{id: :root, render: :root_example},
      %{id: :table, render: :table_example},
      %{id: :tag, render: :tag_example},
      %{id: :text, render: :text_example},
      %{id: :theme_toggle, render: :theme_toggle_example},
      %{id: :build_status, render: :build_status_example},
      %{id: :confirmation_flow, render: :confirmation_flow_example},
      %{id: :instance_summary, render: :instance_summary_example},
      %{id: :organization_header, render: :organization_header_example},
      %{id: :page_heading, render: :page_heading_example},
      %{id: :page_state, render: :page_state_example},
      %{id: :release_provenance, render: :release_provenance_example},
      %{id: :repository_browser, render: :repository_browser_example},
      %{id: :repository_shell, render: :repository_shell_example},
      %{id: :resource_list, render: :resource_list_example},
      %{id: :run_timeline, render: :run_timeline_example},
      %{id: :secret_summary, render: :secret_summary_example},
      %{id: :tab_navigation, render: :tab_navigation_example}
    ]
  end

  attr :id, :atom, required: true

  @doc "Renders one registered example by its stable showcase ID."
  def example(%{id: :action} = assigns), do: action_example(assigns)
  def example(%{id: :app} = assigns), do: app_example(assigns)
  def example(%{id: :breadcrumbs} = assigns), do: breadcrumbs_example(assigns)
  def example(%{id: :button} = assigns), do: button_example(assigns)
  def example(%{id: :flash} = assigns), do: flash_example(assigns)
  def example(%{id: :flash_group} = assigns), do: flash_group_example(assigns)
  def example(%{id: :form_container} = assigns), do: form_container_example(assigns)
  def example(%{id: :frame} = assigns), do: frame_example(assigns)
  def example(%{id: :glyph} = assigns), do: glyph_example(assigns)
  def example(%{id: :header} = assigns), do: header_example(assigns)
  def example(%{id: :icon} = assigns), do: icon_example(assigns)
  def example(%{id: :input} = assigns), do: input_example(assigns)
  def example(%{id: :list} = assigns), do: list_example(assigns)
  def example(%{id: :repository_tree} = assigns), do: repository_tree_example(assigns)
  def example(%{id: :root} = assigns), do: root_example(assigns)
  def example(%{id: :table} = assigns), do: table_example(assigns)
  def example(%{id: :tag} = assigns), do: tag_example(assigns)
  def example(%{id: :text} = assigns), do: text_example(assigns)
  def example(%{id: :theme_toggle} = assigns), do: theme_toggle_example(assigns)
  def example(%{id: :build_status} = assigns), do: build_status_example(assigns)
  def example(%{id: :confirmation_flow} = assigns), do: confirmation_flow_example(assigns)
  def example(%{id: :instance_summary} = assigns), do: instance_summary_example(assigns)
  def example(%{id: :organization_header} = assigns), do: organization_header_example(assigns)
  def example(%{id: :page_heading} = assigns), do: page_heading_example(assigns)
  def example(%{id: :page_state} = assigns), do: page_state_example(assigns)
  def example(%{id: :release_provenance} = assigns), do: release_provenance_example(assigns)
  def example(%{id: :repository_browser} = assigns), do: repository_browser_example(assigns)
  def example(%{id: :repository_shell} = assigns), do: repository_shell_example(assigns)
  def example(%{id: :resource_list} = assigns), do: resource_list_example(assigns)
  def example(%{id: :run_timeline} = assigns), do: run_timeline_example(assigns)
  def example(%{id: :secret_summary} = assigns), do: secret_summary_example(assigns)
  def example(%{id: :tab_navigation} = assigns), do: tab_navigation_example(assigns)

  defp action_example(assigns) do
    ~H"""
    <.action destination="/organizations" aria_label="View organizations">Organizations</.action>
    """
  end

  defp app_example(assigns) do
    assigns = assign(assigns, :flash, %{})

    ~H"""
    <.app
      flash={@flash}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      Application content
    </.app>
    """
  end

  defp breadcrumbs_example(assigns) do
    ~H"""
    <.breadcrumbs id="showcase-breadcrumbs">
      <:item navigate="/organizations">Organizations</:item>
      <:current>Example</:current>
    </.breadcrumbs>
    """
  end

  defp button_example(assigns) do
    ~H"""
    <.button variant="primary">Continue</.button>
    """
  end

  defp flash_example(assigns) do
    ~H"""
    <.flash flash={%{}} kind={:info}>Saved</.flash>
    """
  end

  defp flash_group_example(assigns) do
    assigns = assign(assigns, :flash, %{})

    ~H"""
    <.flash_group flash={@flash} id="showcase-flash-group" />
    """
  end

  defp form_container_example(assigns) do
    assigns = assign(assigns, :form, Phoenix.Component.to_form(%{"name" => ""}, as: :showcase))

    ~H"""
    <.form_container :let={form} for={@form} id="showcase-form" submit="save-showcase">
      <.input field={form[:name]} label="Name" />
      <.button variant="primary">Save</.button>
    </.form_container>
    """
  end

  defp frame_example(assigns) do
    ~H"""
    <.frame as="section" variant={:summary}>Summary</.frame>
    """
  end

  defp glyph_example(assigns) do
    ~H"""
    <.glyph name="hero-check-circle" />
    """
  end

  defp header_example(assigns) do
    ~H"""
    <.header>
      Example header<:subtitle>Supporting text</:subtitle>
    </.header>
    """
  end

  defp icon_example(assigns) do
    ~H"""
    <.icon name="hero-check-circle" />
    """
  end

  defp input_example(assigns) do
    ~H"""
    <.input id="showcase-input" name="showcase" label="Name" value="" />
    """
  end

  defp list_example(assigns) do
    ~H"""
    <.list>
      <:item title="Status">Ready</:item>
    </.list>
    """
  end

  defp repository_tree_example(assigns) do
    assigns = assign(assigns, :tree, showcase_tree())

    ~H"""
    <.repository_tree tree={@tree} current_path="README.md" />
    """
  end

  defp root_example(assigns) do
    ~H"""
    <.root page_title="Showcase" inner_content="Root content" />
    """
  end

  defp table_example(assigns) do
    assigns = assign(assigns, :rows, [%{id: "row", name: "Example"}])

    ~H"""
    <.table id="showcase-table" rows={@rows}>
      <:col :let={row} label="Name">{row.name}</:col>
    </.table>
    """
  end

  defp tag_example(assigns) do
    ~H"""
    <.tag tone="success" dot>Ready</.tag>
    """
  end

  defp text_example(assigns) do
    ~H"""
    <.text as="p" variant={:lede}>Supporting copy</.text>
    """
  end

  defp theme_toggle_example(assigns) do
    ~H"""
    <.theme_toggle />
    """
  end

  defp build_status_example(assigns) do
    ~H"""
    <.build_status id="showcase-build" build_id="build-01" state="succeeded" commit="abc123" />
    """
  end

  defp confirmation_flow_example(assigns) do
    ~H"""
    <.confirmation_flow
      id="showcase-confirmation"
      title="Remove attachment"
      message="Historical provenance will be retained."
      confirm="Remove this attachment?"
      event="remove-attachment"
      label="Remove"
    />
    """
  end

  defp instance_summary_example(assigns) do
    ~H"""
    <.instance_summary
      id="showcase-instance"
      name="Reviewer"
      state="active"
      release="v1"
      attachments={1}
      runs={2}
      destination="/instances/1"
    />
    """
  end

  defp organization_header_example(assigns) do
    assigns = assign(assigns, :organization, %{"id" => "org-01", "name" => "Acme"})

    ~H"""
    <.organization_header organization={@organization} active={:projects} />
    """
  end

  defp page_heading_example(assigns) do
    ~H"""
    <.page_heading
      id="showcase-heading"
      eyebrow="Control plane"
      title="Agents"
      description="Review active installations."
    />
    """
  end

  defp page_state_example(assigns) do
    ~H"""
    <.page_state id="showcase-state" state={:reconnecting} message="Restoring live updates." />
    """
  end

  defp release_provenance_example(assigns) do
    ~H"""
    <.release_provenance
      id="showcase-release"
      version="v1"
      state="published"
      source_commit="abc123"
      build_id="build-01"
      manifest_hash="def456"
    />
    """
  end

  defp repository_browser_example(assigns) do
    ~H"""
    <.repository_browser id="showcase-browser">
      <:tree>Tree</:tree>
      <:content>File content</:content>
    </.repository_browser>
    """
  end

  defp repository_shell_example(assigns) do
    assigns =
      assign(assigns,
        repository: %{
          "name" => "agent-workbench",
          "organization_name" => "Acme",
          "project_name" => "Forge",
          "default_branch" => "refs/heads/main",
          "is_public" => false
        },
        tabs: [
          %{
            key: :files,
            label: "Files",
            icon: "hero-folder",
            destination: "/repositories/1/files"
          }
        ]
      )

    ~H"""
    <.repository_shell
      state={:ready}
      repository={@repository}
      tabs={@tabs}
      active={:files}
      organization_index_destination="/organizations"
      organization_destination="/organizations/1"
      project_destination="/projects/1"
    >
      Repository contents
    </.repository_shell>
    """
  end

  defp resource_list_example(assigns) do
    ~H"""
    <.resource_list id="showcase-resources" layout={:compact} aria_label="Example resources">
      <:header>
        <.text as="span">Name</.text><.text as="span">Count</.text>
      </:header>
      <:row>
        <.frame as="article" id="showcase-resource" variant={:resource_row}>Example</.frame>
      </:row>
    </.resource_list>
    """
  end

  defp run_timeline_example(assigns) do
    assigns =
      assign(assigns, :events, [
        %{
          id: "event-01",
          label: "completed",
          time: "now",
          datetime: "2026-07-31T10:00:00Z",
          detail: "result sealed"
        }
      ])

    ~H"""
    <.run_timeline id="showcase-timeline" events={@events} />
    """
  end

  defp secret_summary_example(assigns) do
    ~H"""
    <.secret_summary
      id="showcase-secret"
      name="deploy_token"
      status="active"
      version={2}
      modes={["brokered"]}
      authority="project"
      bindings={1}
    />
    """
  end

  defp tab_navigation_example(assigns) do
    assigns =
      assign(assigns, :items, [%{key: :one, label: "One", icon: "hero-home", destination: "/one"}])

    ~H"""
    <.tab_navigation id="showcase-tabs" label="Example" items={@items} active={:one} />
    """
  end

  defp showcase_tree(with_destinations \\ true) do
    file = %{name: "README.md", path: "README.md", mode: "100644"}

    file =
      if with_destinations,
        do: Map.put(file, :destination, "/repositories/repo-01/files/README.md?ref=main"),
        else: file

    %{name: "", path: "", file_count: 1, directories: [], files: [file]}
  end
end
