defmodule HephaestusWebWeb.DesignSystem do
  @moduledoc """
  The single public UI facade for the Hephaestus browser application.

  Pages and composites import this module instead of depending on basic-tier
  implementation modules. Bounded component properties are defined centrally
  in `HephaestusWebWeb.DesignSystem.Properties`.
  """

  alias HephaestusWebWeb.DesignSystem.Components.Core
  alias HephaestusWebWeb.DesignSystem.Components.Form
  alias HephaestusWebWeb.DesignSystem.Components.Navigation
  alias HephaestusWebWeb.DesignSystem.Components.Shell
  alias HephaestusWebWeb.DesignSystem.Components.Structure
  alias HephaestusWebWeb.DesignSystem.Interactions
  alias HephaestusWebWeb.DesignSystem.Composites.BuildStatus
  alias HephaestusWebWeb.DesignSystem.Composites.ConfirmationFlow
  alias HephaestusWebWeb.DesignSystem.Composites.InstanceSummary
  alias HephaestusWebWeb.DesignSystem.Composites.OrganizationHeader
  alias HephaestusWebWeb.DesignSystem.Composites.PageHeading
  alias HephaestusWebWeb.DesignSystem.Composites.PageState
  alias HephaestusWebWeb.DesignSystem.Composites.ReleaseProvenance
  alias HephaestusWebWeb.DesignSystem.Composites.RepositoryBrowser
  alias HephaestusWebWeb.DesignSystem.Composites.RepositoryShell
  alias HephaestusWebWeb.DesignSystem.Composites.ResourceList
  alias HephaestusWebWeb.DesignSystem.Composites.RunTimeline
  alias HephaestusWebWeb.DesignSystem.Composites.SecretSummary
  alias HephaestusWebWeb.DesignSystem.Composites.TabNavigation

  defdelegate app(assigns), to: Shell
  defdelegate build_status(assigns), to: BuildStatus
  defdelegate breadcrumbs(assigns), to: Navigation
  defdelegate button(assigns), to: Core
  defdelegate confirmation_flow(assigns), to: ConfirmationFlow
  defdelegate flash(assigns), to: Core
  defdelegate flash_group(assigns), to: Shell
  defdelegate form_container(assigns), to: Form
  defdelegate frame(assigns), to: Structure
  defdelegate glyph(assigns), to: Structure
  defdelegate header(assigns), to: Core
  defdelegate hide(selector), to: Interactions
  defdelegate hide(js, selector), to: Interactions
  defdelegate icon(assigns), to: Core
  defdelegate input(assigns), to: Core
  defdelegate instance_summary(assigns), to: InstanceSummary
  defdelegate list(assigns), to: Core
  defdelegate organization_header(assigns), to: OrganizationHeader
  defdelegate page_heading(assigns), to: PageHeading
  defdelegate page_state(assigns), to: PageState
  defdelegate release_provenance(assigns), to: ReleaseProvenance
  defdelegate repository_browser(assigns), to: RepositoryBrowser
  defdelegate repository_shell(assigns), to: RepositoryShell
  defdelegate root(assigns), to: Shell
  defdelegate resource_list(assigns), to: ResourceList
  defdelegate run_timeline(assigns), to: RunTimeline
  defdelegate repository_tree(assigns), to: Structure
  defdelegate show(selector), to: Interactions
  defdelegate show(js, selector), to: Interactions
  defdelegate action(assigns), to: Structure
  defdelegate table(assigns), to: Core
  defdelegate tag(assigns), to: Navigation
  defdelegate tab_navigation(assigns), to: TabNavigation
  defdelegate text(assigns), to: Structure
  defdelegate theme_toggle(assigns), to: Shell
  defdelegate secret_summary(assigns), to: SecretSummary
  defdelegate translate_error(error), to: Core
  defdelegate translate_errors(errors, field), to: Core

  @doc "Returns the literal public rendering contract consumed by architecture parity checks."
  def catalog do
    [
      %{
        name: :action,
        tier: :component,
        module: Structure,
        function: :action,
        attrs: [
          :id,
          :destination,
          :method,
          :event,
          :value,
          :confirm,
          :disable_with,
          :event_payload,
          :test_id,
          :aria_label,
          :disabled,
          :current,
          :interaction,
          :variant
        ],
        slots: [:inner_block],
        showcase_id: :action,
        a11y_test_id: :action
      },
      %{
        name: :app,
        tier: :component,
        module: Shell,
        function: :app,
        attrs: [:flash, :current_identity, :organizations_destination, :logout_destination],
        slots: [:inner_block],
        showcase_id: :app,
        a11y_test_id: :app
      },
      %{
        name: :breadcrumbs,
        tier: :component,
        module: Navigation,
        function: :breadcrumbs,
        attrs: [:id],
        slots: [:item, :current],
        showcase_id: :breadcrumbs,
        a11y_test_id: :breadcrumbs
      },
      %{
        name: :button,
        tier: :component,
        module: Core,
        function: :button,
        attrs: [
          :href,
          :navigate,
          :patch,
          :method,
          :download,
          :name,
          :value,
          :disabled,
          :variant
        ],
        slots: [:inner_block],
        showcase_id: :button,
        a11y_test_id: :button
      },
      %{
        name: :flash,
        tier: :component,
        module: Core,
        function: :flash,
        attrs: [:id, :flash, :title, :kind, :hidden, :connected, :disconnected],
        slots: [:inner_block],
        showcase_id: :flash,
        a11y_test_id: :flash
      },
      %{
        name: :flash_group,
        tier: :component,
        module: Shell,
        function: :flash_group,
        attrs: [:flash, :id],
        slots: [],
        showcase_id: :flash_group,
        a11y_test_id: :flash_group
      },
      %{
        name: :form_container,
        tier: :component,
        module: Form,
        function: :form_container,
        attrs: [:for, :as, :id, :change, :submit, :layout],
        slots: [:inner_block],
        showcase_id: :form_container,
        a11y_test_id: :form_container
      },
      %{
        name: :frame,
        tier: :component,
        module: Structure,
        function: :frame,
        attrs: [
          :as,
          :id,
          :variant,
          :layout,
          :role,
          :aria_label,
          :aria_live,
          :phx_update,
          :tabindex,
          :open,
          :test_id
        ],
        slots: [:inner_block],
        showcase_id: :frame,
        a11y_test_id: :frame
      },
      %{
        name: :glyph,
        tier: :component,
        module: Structure,
        function: :glyph,
        attrs: [:name, :size, :detail],
        slots: [],
        showcase_id: :glyph,
        a11y_test_id: :glyph
      },
      %{
        name: :header,
        tier: :component,
        module: Core,
        function: :header,
        attrs: [],
        slots: [:inner_block, :subtitle, :actions],
        showcase_id: :header,
        a11y_test_id: :header
      },
      %{
        name: :icon,
        tier: :component,
        module: Core,
        function: :icon,
        attrs: [:name, :size, :treatment],
        slots: [],
        showcase_id: :icon,
        a11y_test_id: :icon
      },
      %{
        name: :input,
        tier: :component,
        module: Core,
        function: :input,
        attrs: [
          :id,
          :name,
          :label,
          :value,
          :type,
          :field,
          :errors,
          :checked,
          :prompt,
          :options,
          :multiple,
          :accept,
          :aria_label,
          :autocomplete,
          :capture,
          :cols,
          :disabled,
          :form,
          :list,
          :max,
          :maxlength,
          :min,
          :minlength,
          :pattern,
          :placeholder,
          :readonly,
          :required,
          :rows,
          :size,
          :step,
          :title
        ],
        slots: [],
        showcase_id: :input,
        a11y_test_id: :input
      },
      %{
        name: :list,
        tier: :component,
        module: Core,
        function: :list,
        attrs: [],
        slots: [:item],
        showcase_id: :list,
        a11y_test_id: :list
      },
      %{
        name: :repository_tree,
        tier: :component,
        module: Structure,
        function: :repository_tree,
        attrs: [:tree, :current_path],
        slots: [],
        showcase_id: :repository_tree,
        a11y_test_id: :repository_tree
      },
      %{
        name: :root,
        tier: :component,
        module: Shell,
        function: :root,
        attrs: [:page_title, :inner_content],
        slots: [],
        showcase_id: :root,
        a11y_test_id: :root
      },
      %{
        name: :table,
        tier: :component,
        module: Core,
        function: :table,
        attrs: [:id, :rows, :row_id, :row_click, :update, :row_item],
        slots: [:col, :action],
        showcase_id: :table,
        a11y_test_id: :table
      },
      %{
        name: :tag,
        tier: :component,
        module: Navigation,
        function: :tag,
        attrs: [:tone, :dot, :variant],
        slots: [:inner_block],
        showcase_id: :tag,
        a11y_test_id: :tag
      },
      %{
        name: :text,
        tier: :component,
        module: Structure,
        function: :text,
        attrs: [:as, :id, :variant, :aria_current, :datetime, :test_id],
        slots: [:inner_block],
        showcase_id: :text,
        a11y_test_id: :text
      },
      %{
        name: :theme_toggle,
        tier: :component,
        module: Shell,
        function: :theme_toggle,
        attrs: [],
        slots: [],
        showcase_id: :theme_toggle,
        a11y_test_id: :theme_toggle
      },
      %{
        name: :build_status,
        tier: :composite,
        module: BuildStatus,
        function: :build_status,
        attrs: [:id, :build_id, :state, :commit],
        slots: [:details],
        showcase_id: :build_status,
        a11y_test_id: :build_status
      },
      %{
        name: :confirmation_flow,
        tier: :composite,
        module: ConfirmationFlow,
        function: :confirmation_flow,
        attrs: [:id, :title, :message, :confirm, :event, :label, :disabled],
        slots: [:cancel],
        showcase_id: :confirmation_flow,
        a11y_test_id: :confirmation_flow
      },
      %{
        name: :instance_summary,
        tier: :composite,
        module: InstanceSummary,
        function: :instance_summary,
        attrs: [:id, :name, :state, :release, :attachments, :runs, :destination],
        slots: [],
        showcase_id: :instance_summary,
        a11y_test_id: :instance_summary
      },
      %{
        name: :organization_header,
        tier: :composite,
        module: OrganizationHeader,
        function: :organization_header,
        attrs: [
          :organization,
          :active,
          :index_destination,
          :projects_destination,
          :secrets_destination
        ],
        slots: [],
        showcase_id: :organization_header,
        a11y_test_id: :organization_header
      },
      %{
        name: :page_heading,
        tier: :composite,
        module: PageHeading,
        function: :page_heading,
        attrs: [:id, :eyebrow, :title, :description, :level],
        slots: [:actions],
        showcase_id: :page_heading,
        a11y_test_id: :page_heading
      },
      %{
        name: :page_state,
        tier: :composite,
        module: PageState,
        function: :page_state,
        attrs: [:id, :state, :title, :message],
        slots: [:inner_block],
        showcase_id: :page_state,
        a11y_test_id: :page_state
      },
      %{
        name: :release_provenance,
        tier: :composite,
        module: ReleaseProvenance,
        function: :release_provenance,
        attrs: [
          :id,
          :version,
          :state,
          :source_commit,
          :build_id,
          :manifest_hash,
          :source_destination,
          :build_destination
        ],
        slots: [],
        showcase_id: :release_provenance,
        a11y_test_id: :release_provenance
      },
      %{
        name: :repository_browser,
        tier: :composite,
        module: RepositoryBrowser,
        function: :repository_browser,
        attrs: [:id],
        slots: [:navigation, :tree, :content],
        showcase_id: :repository_browser,
        a11y_test_id: :repository_browser
      },
      %{
        name: :repository_shell,
        tier: :composite,
        module: RepositoryShell,
        function: :repository_shell,
        attrs: [
          :state,
          :repository,
          :tabs,
          :active,
          :organization_index_destination,
          :organization_destination,
          :project_destination
        ],
        slots: [:inner_block],
        showcase_id: :repository_shell,
        a11y_test_id: :repository_shell
      },
      %{
        name: :resource_list,
        tier: :composite,
        module: ResourceList,
        function: :resource_list,
        attrs: [:id, :layout, :update, :aria_label],
        slots: [:header, :row, :empty, :inner_block],
        showcase_id: :resource_list,
        a11y_test_id: :resource_list
      },
      %{
        name: :run_timeline,
        tier: :composite,
        module: RunTimeline,
        function: :run_timeline,
        attrs: [:id, :events],
        slots: [],
        showcase_id: :run_timeline,
        a11y_test_id: :run_timeline
      },
      %{
        name: :secret_summary,
        tier: :composite,
        module: SecretSummary,
        function: :secret_summary,
        attrs: [:id, :name, :status, :version, :modes, :authority, :bindings],
        slots: [:controls],
        showcase_id: :secret_summary,
        a11y_test_id: :secret_summary
      },
      %{
        name: :tab_navigation,
        tier: :composite,
        module: TabNavigation,
        function: :tab_navigation,
        attrs: [:id, :label, :items, :active],
        slots: [],
        showcase_id: :tab_navigation,
        a11y_test_id: :tab_navigation
      }
    ]
  end
end
