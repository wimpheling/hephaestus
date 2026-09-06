import {createHighlighterCore} from "shiki/core"
import {createOnigurumaEngine} from "shiki/engine/oniguruma"
import wasm from "shiki/wasm"
import elixir from "@shikijs/langs/elixir"
import json from "@shikijs/langs/json"
import markdown from "@shikijs/langs/markdown"
import rust from "@shikijs/langs/rust"
import shell from "@shikijs/langs/shellscript"
import sql from "@shikijs/langs/sql"
import toml from "@shikijs/langs/toml"
import yaml from "@shikijs/langs/yaml"
import githubDark from "@shikijs/themes/github-dark"
import githubLight from "@shikijs/themes/github-light"

const languages = new Set(["elixir", "json", "markdown", "rust", "shell", "sql", "toml", "yaml"])
const highlighter = createHighlighterCore({
  themes: [githubDark, githubLight],
  langs: [elixir, json, markdown, rust, shell, sql, toml, yaml],
  engine: createOnigurumaEngine(wasm),
})

const theme = () => document.documentElement.getAttribute("data-theme") === "dark" ? "github-dark" : "github-light"

const codeFor = element => [...element.querySelectorAll(".file-source-content")]
  .map(line => line.textContent)
  .join("\n")

const render = async element => {
  const language = element.dataset.sourceLanguage
  if (!languages.has(language) || element.dataset.sourceHighlighted === "true") return

  try {
    const tokens = await highlighter.then(instance => instance.codeToTokens(codeFor(element), {lang: language, theme: theme()}).tokens)
    const lines = element.querySelectorAll(".file-source-content")

    tokens.forEach((tokenLine, index) => {
      const line = lines[index]
      if (!line) return
      const fragment = document.createDocumentFragment()

      tokenLine.forEach(token => {
        const span = document.createElement("span")
        span.className = "source-token"
        span.style.color = token.color
        span.textContent = token.content
        fragment.append(span)
      })

      line.replaceChildren(fragment)
    })

    element.dataset.sourceHighlighted = "true"
  } catch (_error) {
    // Escaped server-rendered source remains the intentional fallback.
  }
}

export const SourceHighlight = {
  mounted() {
    render(this.el)
  },
  updated() {
    this.el.dataset.sourceHighlighted = ""
    render(this.el)
  },
}

export const installSourceHighlight = () => {
  window.addEventListener("phx:set-theme", () => {
    document.querySelectorAll("[data-source-highlighted='true']").forEach(element => {
      element.dataset.sourceHighlighted = ""
      render(element)
    })
  })
}
