defmodule HephaestusWebWeb.DesignSystem.Components.Structure do
  @moduledoc """
  Bounded semantic structure used by public design-system composites.

  This basic-tier module owns the raw markup and maps finite presentation
  properties to implementation classes. Composite callers never pass CSS.
  """

  use Phoenix.Component

  attr :as, :string,
    default: "div",
    values:
      ~w(article aside details div dl footer header i li main nav ol p section span summary ul)

  attr :id, :string, default: nil

  attr :variant, :atom,
    required: true,
    values: [
      :organization_header,
      :organization_mark,
      :organization_body,
      :tabs,
      :page_heading,
      :page_heading_copy,
      :page_heading_actions,
      :resource_list,
      :resource_heading,
      :resource_empty,
      :resource_row,
      :page_state,
      :summary,
      :summary_header,
      :summary_body,
      :metadata,
      :timeline,
      :timeline_item,
      :timeline_dot,
      :confirmation,
      :repository_browser,
      :tree_level,
      :tree_directory,
      :tree_summary,
      :tree_empty,
      :panel,
      :section_heading,
      :table,
      :table_head,
      :table_row,
      :metric_grid,
      :metric,
      :form_group,
      :form_actions,
      :command_grid,
      :command_row,
      :two_column,
      :proposal,
      :proposal_heading,
      :commit_flow,
      :review_actions,
      :review_grid,
      :timeline_panel,
      :artifact_panel,
      :artifact_list,
      :artifact_row,
      :run_hero,
      :run_title,
      :control_bar,
      :instance_overview,
      :secret_record,
      :binding_form,
      :inline_command,
      :resource_primary,
      :resource_detail,
      :resource_controls,
      :branch_toolbar,
      :repository_list,
      :hero_panel,
      :system_health,
      :status_dot,
      :organization_grid,
      :organization_card_mark,
      :organization_card_copy,
      :arrow,
      :workspace_heading,
      :list_subheading,
      :resource_row_tall,
      :repository_icon,
      :landing_shell,
      :landing_navigation,
      :landing_hero,
      :landing_copy,
      :landing_brand_mark,
      :landing_console,
      :console_top,
      :console_lights,
      :console_light,
      :console_body,
      :console_line,
      :console_line_active,
      :console_foot,
      :landing_footer
    ]

  attr :layout, :atom,
    default: :default,
    values: [:default, :compact, :projects, :secrets, :grants]

  attr :role, :string, default: nil, values: [nil, "alert", "status"]
  attr :aria_label, :string, default: nil
  attr :aria_live, :string, default: nil, values: [nil, "polite", "assertive"]
  attr :phx_update, :string, default: nil, values: [nil, "stream", "ignore"]
  attr :test_id, :string, default: nil
  attr :tabindex, :integer, default: nil, values: [nil, 0]
  attr :open, :boolean, default: false
  slot :inner_block

  @doc "Renders a semantic container with a bounded visual role."
  def frame(assigns) do
    assigns = assign(assigns, :classes, frame_classes(assigns.variant, assigns.layout))

    ~H"""
    <.dynamic_tag tag_name={@as} {frame_attributes(assigns)}>
      {render_slot(@inner_block)}
    </.dynamic_tag>
    """
  end

  attr :as, :string,
    default: "span",
    values: ~w(b code dd dt em h1 h2 h3 p pre small span strong time)

  attr :id, :string, default: nil

  attr :variant, :atom,
    default: :body,
    values: [
      :body,
      :eyebrow,
      :lede,
      :title,
      :subtitle,
      :muted,
      :mono,
      :sr_only,
      :field_help,
      :empty,
      :result_message,
      :status,
      :landing_emphasis
    ]

  attr :aria_current, :string, default: nil, values: [nil, "page", "step"]
  attr :datetime, :string, default: nil
  attr :test_id, :string, default: nil
  slot :inner_block, required: true

  @doc "Renders bounded typography with a caller-selected semantic element."
  def text(assigns) do
    ~H"""
    <.dynamic_tag tag_name={@as} {text_attributes(assigns)}>
      {render_slot(@inner_block)}
    </.dynamic_tag>
    """
  end

  attr :id, :string, default: nil
  attr :destination, :string, default: nil
  attr :method, :string, default: nil, values: [nil, "delete", "get", "patch", "post", "put"]

  attr :event, :string,
    default: nil,
    values: [
      nil,
      "accept-secret-import",
      "bind-secret",
      "create-attachment",
      "create-secret",
      "create-update",
      "grant-secret",
      "import-agent",
      "purge-secret",
      "recover-update",
      "remove-attachment",
      "revise-instance",
      "revoke-secret",
      "rotate-secret",
      "set-attachment",
      "set-secret-enabled",
      "set-draft-version",
      "publish-release"
    ]

  attr :value, :string, default: nil
  attr :confirm, :string, default: nil
  attr :disable_with, :string, default: nil
  attr :event_payload, :map, default: %{}
  attr :test_id, :string, default: nil
  attr :aria_label, :string, default: nil
  attr :disabled, :boolean, default: false
  attr :current, :boolean, default: false

  attr :interaction, :atom,
    default: :navigate,
    values: [:navigate, :patch, :href, :event, :submit]

  attr :variant, :atom,
    default: :text,
    values: [
      :text,
      :tab,
      :primary,
      :secondary,
      :danger,
      :compact,
      :danger_compact,
      :organization_card,
      :resource_row,
      :brand,
      :landing_primary,
      :tree_file
    ]

  slot :inner_block, required: true

  @doc "Renders a link or button from a finite interaction vocabulary."
  def action(%{interaction: interaction} = assigns)
      when interaction in [:navigate, :patch, :href] do
    assigns = validate_event_payload!(assigns)

    ~H"""
    <.link
      id={@id}
      navigate={@interaction == :navigate && @destination}
      patch={@interaction == :patch && @destination}
      href={@interaction == :href && @destination}
      method={@method}
      class={action_class(@variant, @current)}
      aria-current={@current && "page"}
      aria-label={@aria_label}
      data-testid={@test_id}
    >
      {render_slot(@inner_block)}
    </.link>
    """
  end

  def action(assigns) do
    assigns = validate_event_payload!(assigns)

    ~H"""
    <button
      id={@id}
      type={if(@interaction == :submit, do: "submit", else: "button")}
      phx-click={@event}
      value={@value}
      data-confirm={@confirm}
      data-testid={@test_id}
      aria-label={@aria_label}
      phx-disable-with={@disable_with}
      phx-value-action={@event_payload[:action]}
      phx-value-attachment_id={@event_payload[:attachment_id]}
      phx-value-enabled={@event_payload[:enabled]}
      phx-value-kind={@event_payload[:kind]}
      phx-value-secret_id={@event_payload[:secret_id]}
      phx-value-update_id={@event_payload[:update_id]}
      disabled={@disabled}
      class={action_class(@variant, @current)}
    >
      {render_slot(@inner_block)}
    </button>
    """
  end

  attr :name, :string, required: true
  attr :size, :atom, default: :medium, values: [:small, :medium, :large]
  attr :detail, :atom, default: :default, values: [:default, :chevron]

  @doc "Renders an icon with bounded size and detail treatment."
  def glyph(%{name: "hero-" <> _rest} = assigns) do
    ~H"""
    <span
      class={[@name, glyph_size(@size), @detail == :chevron && "tree-chevron"]}
      aria-hidden="true"
    />
    """
  end

  attr :tree, :map, required: true
  attr :current_path, :string, default: nil

  @doc "Renders a repository tree whose destinations were prepared by its composite."
  def repository_tree(assigns) do
    ~H"""
    <div id="repository-file-tree" class="file-tree">
      <.tree_node node={@tree} current_path={@current_path} />
      <div :if={@tree.directories == [] and @tree.files == []} class="file-tree-empty">
        This branch has no files.
      </div>
    </div>
    """
  end

  attr :node, :map, required: true
  attr :current_path, :string, default: nil

  defp tree_node(assigns) do
    ~H"""
    <div class="tree-level">
      <details
        :for={directory <- @node.directories}
        id={"tree-directory-#{tree_id(directory.path)}"}
        open={directory_open?(directory.path, @current_path)}
      >
        <summary>
          <.glyph name="hero-chevron-right" size={:small} detail={:chevron} />
          <.glyph name="hero-folder" />
          <span>{directory.name}</span>
        </summary>
        <.tree_node node={directory} current_path={@current_path} />
      </details>
      <.link
        :for={file <- @node.files}
        id={"tree-file-#{tree_id(file.path)}"}
        navigate={file.destination}
        class={["tree-file", file.path == @current_path && "active"]}
        title={file.path}
      >
        <.glyph name={if(file.mode == "120000", do: "hero-link", else: "hero-document")} />
        <span>{file.name}</span>
      </.link>
    </div>
    """
  end

  defp frame_classes(:organization_header, _layout), do: "organization-hero"
  defp frame_classes(:organization_mark, _layout), do: "org-mark"
  defp frame_classes(:organization_body, _layout), do: "org-copy"
  defp frame_classes(:tabs, _layout), do: "repository-tabs"
  defp frame_classes(:page_heading, _layout), do: "section-heading spacious"
  defp frame_classes(:page_heading_copy, _layout), do: nil
  defp frame_classes(:page_heading_actions, _layout), do: "page-actions"

  defp frame_classes(:resource_list, layout),
    do: ["resource-list", resource_layout_class(layout)]

  defp frame_classes(:resource_heading, _layout), do: "resource-list-heading"
  defp frame_classes(:resource_empty, _layout), do: "resource-list-empty empty-copy"
  defp frame_classes(:resource_row, _layout), do: "resource-list-row"
  defp frame_classes(:page_state, _layout), do: "empty-state"
  defp frame_classes(:summary, _layout), do: "panel"
  defp frame_classes(:summary_header, _layout), do: "panel-heading"
  defp frame_classes(:summary_body, _layout), do: nil
  defp frame_classes(:metadata, _layout), do: "metadata-grid"
  defp frame_classes(:timeline, _layout), do: "timeline"
  defp frame_classes(:timeline_item, _layout), do: nil
  defp frame_classes(:timeline_dot, _layout), do: "timeline-dot"
  defp frame_classes(:confirmation, _layout), do: "danger-confirmation"
  defp frame_classes(:repository_browser, _layout), do: "repository-browser"
  defp frame_classes(:tree_level, _layout), do: "tree-level"
  defp frame_classes(:tree_directory, _layout), do: nil
  defp frame_classes(:tree_summary, _layout), do: nil
  defp frame_classes(:tree_empty, _layout), do: "file-tree-empty"
  defp frame_classes(:panel, _layout), do: "panel"
  defp frame_classes(:section_heading, _layout), do: "section-heading"
  defp frame_classes(:table, _layout), do: "repository-table"
  defp frame_classes(:table_head, _layout), do: "table-head"
  defp frame_classes(:table_row, _layout), do: "repo-row"
  defp frame_classes(:metric_grid, _layout), do: "metric-grid"
  defp frame_classes(:metric, _layout), do: "metric"
  defp frame_classes(:form_group, _layout), do: "panel form-page-panel"
  defp frame_classes(:form_actions, _layout), do: "form-page-actions"
  defp frame_classes(:command_grid, _layout), do: "command-grid"
  defp frame_classes(:command_row, _layout), do: "command-row"
  defp frame_classes(:two_column, _layout), do: "two-column-controls"
  defp frame_classes(:proposal, _layout), do: "proposal-card"
  defp frame_classes(:proposal_heading, _layout), do: "proposal-heading"
  defp frame_classes(:commit_flow, _layout), do: "commit-flow"
  defp frame_classes(:review_actions, _layout), do: "review-actions"
  defp frame_classes(:review_grid, _layout), do: "review-grid"
  defp frame_classes(:timeline_panel, _layout), do: "panel timeline-panel"
  defp frame_classes(:artifact_panel, _layout), do: "panel artifact-panel"
  defp frame_classes(:artifact_list, _layout), do: "artifact-list"
  defp frame_classes(:artifact_row, _layout), do: "artifact-row"
  defp frame_classes(:run_hero, _layout), do: "run-hero"
  defp frame_classes(:run_title, _layout), do: "run-title-line"
  defp frame_classes(:control_bar, _layout), do: "control-bar"
  defp frame_classes(:instance_overview, _layout), do: "instance-overview"
  defp frame_classes(:secret_record, _layout), do: "secret-record"
  defp frame_classes(:binding_form, _layout), do: "binding-form"
  defp frame_classes(:inline_command, _layout), do: "inline-command"
  defp frame_classes(:resource_primary, _layout), do: "resource-primary"
  defp frame_classes(:resource_detail, _layout), do: "resource-detail"
  defp frame_classes(:resource_controls, _layout), do: "resource-controls"
  defp frame_classes(:branch_toolbar, _layout), do: "branch-toolbar"
  defp frame_classes(:repository_list, _layout), do: "repository-list"
  defp frame_classes(:hero_panel, _layout), do: "hero-panel"
  defp frame_classes(:system_health, _layout), do: "system-health"
  defp frame_classes(:status_dot, _layout), do: "status-dot"
  defp frame_classes(:organization_grid, _layout), do: "org-grid"
  defp frame_classes(:organization_card_mark, _layout), do: "org-mark"
  defp frame_classes(:organization_card_copy, _layout), do: "org-copy"
  defp frame_classes(:arrow, _layout), do: "arrow"
  defp frame_classes(:workspace_heading, _layout), do: "section-heading workspace-heading"
  defp frame_classes(:list_subheading, _layout), do: "section-heading list-subheading"
  defp frame_classes(:resource_row_tall, _layout), do: "resource-list-row resource-list-row-tall"
  defp frame_classes(:repository_icon, _layout), do: "repo-icon"
  defp frame_classes(:landing_shell, _layout), do: "landing-shell"
  defp frame_classes(:landing_navigation, _layout), do: "landing-nav"
  defp frame_classes(:landing_hero, _layout), do: "landing-hero"
  defp frame_classes(:landing_copy, _layout), do: "landing-copy"
  defp frame_classes(:landing_brand_mark, _layout), do: "brand-mark"
  defp frame_classes(:landing_console, _layout), do: "landing-console"
  defp frame_classes(:console_top, _layout), do: "console-top"
  defp frame_classes(:console_lights, _layout), do: nil
  defp frame_classes(:console_light, _layout), do: nil
  defp frame_classes(:console_body, _layout), do: "console-body"
  defp frame_classes(:console_line, _layout), do: nil
  defp frame_classes(:console_line_active, _layout), do: "active"
  defp frame_classes(:console_foot, _layout), do: "console-foot"
  defp frame_classes(:landing_footer, _layout), do: "landing-footer"

  defp resource_layout_class(:default), do: "resource-columns-default"
  defp resource_layout_class(:compact), do: "resource-columns-compact"
  defp resource_layout_class(:projects), do: "resource-columns-projects"
  defp resource_layout_class(:secrets), do: "resource-columns-secrets"
  defp resource_layout_class(:grants), do: "resource-columns-grants"

  defp text_class(:body), do: nil
  defp text_class(:eyebrow), do: "eyebrow"
  defp text_class(:lede), do: "lede"
  defp text_class(:title), do: "ds-title"
  defp text_class(:subtitle), do: "ds-subtitle"
  defp text_class(:muted), do: "ds-muted"
  defp text_class(:mono), do: "mono"
  defp text_class(:sr_only), do: "sr-only"
  defp text_class(:field_help), do: "field-help"
  defp text_class(:empty), do: "empty-copy"
  defp text_class(:result_message), do: "result-message"
  defp text_class(:status), do: "status-live"
  defp text_class(:landing_emphasis), do: "landing-emphasis"

  defp action_class(:text, _current), do: "ds-text-action"
  defp action_class(:tab, false), do: "repository-tab"
  defp action_class(:tab, true), do: "repository-tab active"
  defp action_class(:primary, _current), do: "button primary"
  defp action_class(:secondary, _current), do: "button secondary"
  defp action_class(:danger, _current), do: "button danger"
  defp action_class(:compact, _current), do: "button secondary compact"
  defp action_class(:danger_compact, _current), do: "button danger compact"
  defp action_class(:organization_card, _current), do: "org-card"
  defp action_class(:resource_row, _current), do: "resource-list-row"
  defp action_class(:brand, _current), do: "brand"
  defp action_class(:landing_primary, _current), do: "button primary landing-button"
  defp action_class(:tree_file, false), do: "tree-file"
  defp action_class(:tree_file, true), do: "tree-file active"

  defp glyph_size(:small), do: "size-3"
  defp glyph_size(:medium), do: "size-4"
  defp glyph_size(:large), do: "size-8"

  defp directory_open?(_directory_path, nil), do: false

  defp directory_open?(directory_path, current_path),
    do: String.starts_with?(current_path, directory_path <> "/")

  defp tree_id(path) do
    :crypto.hash(:sha256, path)
    |> Base.url_encode64(padding: false)
    |> binary_part(0, 12)
  end

  defp frame_attributes(assigns) do
    %{
      id: assigns.id,
      class: assigns.classes,
      role: assigns.role,
      "aria-label": assigns.aria_label,
      "aria-live": assigns.aria_live,
      "phx-update": assigns.phx_update,
      "data-testid": assigns.test_id,
      tabindex: assigns.tabindex,
      open: assigns.as == "details" && assigns.open
    }
    |> Enum.reject(fn {_key, value} -> value in [nil, false] end)
    |> Map.new()
  end

  defp text_attributes(assigns) do
    %{
      id: assigns.id,
      class: text_class(assigns.variant),
      "aria-current": assigns.aria_current,
      "data-testid": assigns.test_id,
      datetime: assigns.as == "time" && assigns.datetime
    }
    |> Enum.reject(fn {_key, value} -> value in [nil, false] end)
    |> Map.new()
  end

  @event_payload_keys [:action, :attachment_id, :enabled, :kind, :secret_id, :update_id]

  defp validate_event_payload!(assigns) do
    payload =
      Enum.reduce(assigns.event_payload, %{}, fn {key, value}, payload ->
        normalized_key = normalize_event_payload_key(key)

        if normalized_key in @event_payload_keys do
          Map.put(payload, normalized_key, value)
        else
          raise ArgumentError,
                "unsupported event payload key #{inspect(key)}; allowed keys: #{inspect(@event_payload_keys)}"
        end
      end)

    assign(assigns, :event_payload, payload)
  end

  defp normalize_event_payload_key(key) when is_atom(key), do: key

  defp normalize_event_payload_key(key) when is_binary(key) do
    Enum.find(@event_payload_keys, key, &(Atom.to_string(&1) == key))
  end

  defp normalize_event_payload_key(key), do: key
end
