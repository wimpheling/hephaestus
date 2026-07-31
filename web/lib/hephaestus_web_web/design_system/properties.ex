defmodule HephaestusWebWeb.DesignSystem.Properties do
  @moduledoc """
  Central bounded visual vocabulary for design-system implementations.

  Public components expose these finite properties rather than accepting
  caller-authored styling. The temporary legacy class attributes on migrated
  primitives remain visible to the migration checker until their callers have
  moved to this vocabulary.
  """

  @tones [:neutral, :accent, :success, :warning, :danger]
  @sizes [:small, :medium, :large]
  @densities [:compact, :comfortable, :spacious]
  @spacings [:none, :small, :medium, :large]
  @alignments [:start, :center, :end, :between]
  @widths [:content, :full]
  @states [:idle, :loading, :disabled, :selected]
  @interactions [:none, :navigate, :action, :submit]

  @type tone :: :neutral | :accent | :success | :warning | :danger
  @type size :: :small | :medium | :large
  @type density :: :compact | :comfortable | :spacious
  @type spacing :: :none | :small | :medium | :large
  @type alignment :: :start | :center | :end | :between
  @type width :: :content | :full
  @type state :: :idle | :loading | :disabled | :selected
  @type interaction :: :none | :navigate | :action | :submit

  @doc false
  def tones, do: @tones

  @doc false
  def sizes, do: @sizes

  @doc false
  def densities, do: @densities

  @doc false
  def spacings, do: @spacings

  @doc false
  def alignments, do: @alignments

  @doc false
  def widths, do: @widths

  @doc false
  def states, do: @states

  @doc false
  def interactions, do: @interactions
end
