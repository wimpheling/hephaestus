const fragmentPattern = /^[A-Za-z0-9_-]{43}$/
const generationHostPattern = /^g-[0-9a-f]{32}$/
const themes = new Set(["light", "dark"])
const states = new Set(["loading", "ready", "unavailable", "revoked", "terminated"])
const loadingTimeoutMs = 10_000
const closeMessages = {
  access_revoked: "This installed UI is no longer available.",
  unavailable: "This installed UI is no longer available.",
  active_generation_changed: "This installed UI changed. Refresh to launch the current version.",
  timeout: "The installed UI did not respond.",
  terminated: "The installed UI has been closed.",
}

const configuredPort = value => {
  if (value === undefined || value === null || value === "") return 443

  const port = Number(value)
  return Number.isInteger(port) && port >= 1 && port <= 65_535 ? port : null
}

export const safeBootstrapUrl = (value, namespace, publicPort = 443) => {
  if (typeof value !== "string" || !namespace) return null

  const expectedPort = configuredPort(publicPort)
  if (expectedPort === null) return null

  let url
  try {
    url = new URL(value)
  } catch (_error) {
    return null
  }

  const hostname = url.hostname.toLowerCase()
  const expectedSuffix = `.${namespace.toLowerCase()}`
  const hostLabel = hostname.endsWith(expectedSuffix)
    ? hostname.slice(0, -expectedSuffix.length)
    : ""
  const actualPort = url.port === "" ? 443 : Number(url.port)
  const expectedThemeQuery = ["?heph_theme=light", "?heph_theme=dark"]

  if (url.protocol !== "https:" || url.username || url.password) return null
  if (actualPort !== expectedPort) return null
  if (!generationHostPattern.test(hostLabel)) return null
  if (url.pathname !== "/_heph/bootstrap" || !expectedThemeQuery.includes(url.search)) return null
  if (!fragmentPattern.test(url.hash.slice(1))) return null

  return url
}

const setStatus = (element, value) => {
  const status = element.querySelector("[data-ui-status]")
  if (status) status.textContent = value
}

const setTerminalStatus = (element, value) => {
  const status = element.querySelector("[data-ui-terminal-status]")
  if (!status) return

  status.textContent = value
  status.classList.remove("hidden")
}

const clearTerminalStatus = element => {
  const status = element.querySelector("[data-ui-terminal-status]")
  if (status) {
    status.textContent = ""
    status.classList.add("hidden")
  }
}

const documentTheme = () => {
  const theme = document.documentElement?.dataset?.theme
  return themes.has(theme) ? theme : "light"
}

const withTheme = (url, theme) => {
  const themed = new URL(url.toString())
  themed.searchParams.set("heph_theme", theme)
  return themed
}

export const InstalledUiNavigation = {
  mounted() {
    this.frame = this.el.querySelector("[data-ui-frame]")
    this.closeButton = this.el.querySelector("[data-ui-close]")
    this.statusRoot = this.el.parentElement || this.el
    this.expectedOrigin = null
    this.loadingTimer = null
    this.activeInstallationId = null

    this.clearLoadingTimer = () => {
      if (this.loadingTimer !== null) {
        window.clearTimeout(this.loadingTimer)
        this.loadingTimer = null
      }
    }

    this.sendTheme = () => {
      if (!this.expectedOrigin || !this.frame.contentWindow) return

      this.frame.contentWindow.postMessage(
        {type: "heph-ui-theme", theme: documentTheme()},
        this.expectedOrigin,
      )
    }

    this.close = detail => {
      const reason = typeof detail === "string" ? detail : detail?.reason
      const message = closeMessages[reason] || null
      this.clearLoadingTimer()
      this.expectedOrigin = null
      this.activeInstallationId = null
      this.frame.removeAttribute("src")
      this.el.classList.add("hidden")
      setStatus(this.el, reason === "user_closed" ? "Closed" : message || "Unavailable")
      clearTerminalStatus(this.statusRoot)
      if (message) setTerminalStatus(this.statusRoot, message)

      const launchButtons = this.statusRoot.querySelectorAll?.("[data-ui-installation] button") || []
      const button = Array.from(launchButtons).find(candidate =>
        candidate.getAttribute?.("phx-value-id") === this.closedInstallationId,
      )
      ;(button || launchButtons[0])?.focus?.()
    }

    this.closeButton?.addEventListener("click", () => {
      this.closedInstallationId = this.activeInstallationId
      this.pushEvent?.("close-installed-ui", {})
      this.close({reason: "user_closed"})
    })
    this.messageHandler = event => {
      if (!this.expectedOrigin || event.origin !== this.expectedOrigin || event.source !== this.frame.contentWindow) return

      const data = event.data
      if (!data || typeof data !== "object") return

      if (data.type === "heph-ui-ready") {
        this.clearLoadingTimer()
        setStatus(this.el, "Ready")
        this.sendTheme()
        return
      }

      if (data.type === "heph-ui-state" && states.has(data.state)) {
        if (["unavailable", "revoked", "terminated"].includes(data.state)) {
          const reason =
            data.state === "revoked"
              ? "access_revoked"
              : data.state === "unavailable"
                ? "unavailable"
                : "terminated"
          this.close({reason})
          return
        }

        if (data.state === "ready") this.clearLoadingTimer()
        setStatus(this.el, data.state === "ready" ? "Ready" : data.state)
        return
      }

      if (data.type === "heph-ui-appearance" && themes.has(data.theme)) {
        this.el.dataset.uiTheme = data.theme
      }
    }

    window.addEventListener("message", this.messageHandler)
    if (typeof MutationObserver !== "undefined") {
      this.themeObserver = new MutationObserver(() => this.sendTheme())
      this.themeObserver.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-theme"],
      })
    }

    this.handleEvent("ui-browser-launch", detail => this.launch(detail))
    this.handleEvent("ui-browser-close", detail => this.close(detail))
  },

  updated() {},

  destroyed() {
    this.clearLoadingTimer?.()
    this.themeObserver?.disconnect()
    window.removeEventListener("message", this.messageHandler)
  },

  launch(detail) {
    const url = safeBootstrapUrl(
      detail?.url,
      this.el.dataset.uiNamespace,
      this.el.dataset.uiPort,
    )
    const mode = detail?.mode
    if (!url || !["iframe", "full_page"].includes(mode)) {
      setStatus(this.el, "Unavailable")
      setTerminalStatus(this.statusRoot, "This installed UI is unavailable.")
      return
    }

    this.closedInstallationId = detail?.installation_id || null
    this.activeInstallationId = detail?.installation_id || null
    const themedUrl = withTheme(url, documentTheme())
    if (mode === "full_page") {
      window.location.replace(themedUrl.toString())
      return
    }

    this.clearLoadingTimer()
    clearTerminalStatus(this.statusRoot)
    this.expectedOrigin = themedUrl.origin
    this.el.classList.remove("hidden")
    this.frame.hidden = false
    setStatus(this.el, "Loading")
    this.frame.src = themedUrl.toString()
    this.loadingTimer = window.setTimeout(
      () => this.close({reason: "timeout"}),
      loadingTimeoutMs,
    )
  },
}
