defmodule HephaestusWebWeb.DesignSystem.Pages.HomePage do
  @moduledoc "Pure presentation boundary for the public landing page."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem,
    only: [action: 1, flash_group: 1, frame: 1, tag: 1, text: 1]

  @states [:ready]

  attr :state, :atom, default: :ready, values: @states
  attr :flash, :map, required: true

  @doc "Renders the public landing page through the design-system facade."
  def home_page(assigns) do
    ~H"""
    <.flash_group flash={@flash} />
    <.frame :if={@state == :ready} as="main" variant={:landing_shell}>
      <.frame as="nav" variant={:landing_navigation} aria_label="Public">
        <.action interaction={:href} destination="/" variant={:brand}>
          <.frame as="span" variant={:landing_brand_mark}>H</.frame>
          <.text as="span">HEPHAESTUS</.text>
        </.action>
        <.tag>Runnable POC</.tag>
      </.frame>

      <.frame as="section" variant={:landing_hero}>
        <.frame variant={:landing_copy}>
          <.text as="p" variant={:eyebrow}>Git forge · agent runtime · human control</.text>
          <.text as="h1">
            Code enters.<.text as="em" variant={:landing_emphasis}>Intent emerges.</.text>
          </.text>
          <.text as="p">
            Push an exact commit. Watch an isolated agent work live. Review its durable result
            before a controlled fast-forward reaches your branch.
          </.text>
          <.action
            interaction={:href}
            destination="/login"
            variant={:landing_primary}
            test_id="oidc-login"
          >
            Sign in with OIDC
            <.frame as="span" variant={:summary_body}>→</.frame>
          </.action>
          <.text as="small">Identity and authorization are enforced inside PostgreSQL.</.text>
        </.frame>

        <.frame variant={:landing_console} aria_label="Hephaestus execution flow">
          <.frame variant={:console_top}>
            <.frame as="span" variant={:console_lights}>
              <.frame as="i" variant={:console_light} />
              <.frame as="i" variant={:console_light} />
              <.frame as="i" variant={:console_light} />
            </.frame>
            <.text as="code">run / 8d71e42a</.text>
            <.text as="b">LIVE</.text>
          </.frame>
          <.frame variant={:console_body}>
            <.console_line number="01" label="receive.accepted" detail="refs/heads/main" />
            <.console_line number="02" label="workspace.materialized" detail="4fa2c31" />
            <.console_line number="03" label="vm.running" detail="isolated · 2 vCPU" />
            <.console_line number="04" label="agent.editing" detail="+ 47 / − 12" active />
            <.console_line number="05" label="result.sealed" detail="waiting" />
          </.frame>
          <.frame variant={:console_foot}>
            <.text as="span">Exact input parent</.text><.text as="code">4fa2c31 → pending</.text>
          </.frame>
        </.frame>
      </.frame>

      <.frame as="footer" variant={:landing_footer}>
        <.text as="span">Immutable source</.text><.text as="span">Durable events</.text><.text as="span">
          CAS publication
        </.text>
      </.frame>
    </.frame>
    """
  end

  attr :number, :string, required: true
  attr :label, :string, required: true
  attr :detail, :string, required: true
  attr :active, :boolean, default: false

  defp console_line(assigns) do
    ~H"""
    <.frame as="p" variant={if(@active, do: :console_line_active, else: :console_line)}>
      <.text as="span">{@number}</.text><.text as="strong">{@label}</.text><.text as="code">
        {@detail}
      </.text>
    </.frame>
    """
  end
end
