const themeStorageKey = "phx:theme"
const systemTheme = "system"
const supportedThemes = new Set([systemTheme, "light", "dark"])
const colorScheme = window.matchMedia("(prefers-color-scheme: dark)")

const preferredSystemTheme = () => colorScheme.matches ? "dark" : "light"

const boundedTheme = theme => supportedThemes.has(theme) ? theme : systemTheme

const applyTheme = requestedTheme => {
  const theme = boundedTheme(requestedTheme)

  if (theme === systemTheme) {
    localStorage.removeItem(themeStorageKey)
    document.documentElement.setAttribute("data-theme", preferredSystemTheme())
    document.documentElement.setAttribute("data-theme-source", systemTheme)
    return
  }

  localStorage.setItem(themeStorageKey, theme)
  document.documentElement.setAttribute("data-theme", theme)
  document.documentElement.setAttribute("data-theme-source", "user")
}

export const installTheme = () => {
  if (!document.documentElement.hasAttribute("data-theme")) {
    applyTheme(localStorage.getItem(themeStorageKey))
  }

  window.addEventListener("storage", event => {
    if (event.key === themeStorageKey) applyTheme(event.newValue)
  })

  window.addEventListener("phx:set-theme", event => {
    applyTheme(event.target.dataset.phxTheme)
  })

  colorScheme.addEventListener("change", () => {
    if (document.documentElement.getAttribute("data-theme-source") === systemTheme) {
      document.documentElement.setAttribute("data-theme", preferredSystemTheme())
    }
  })
}
