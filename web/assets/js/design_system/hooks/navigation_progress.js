import topbar from "../../../vendor/topbar"

const progressDelay = 300
const progressColorToken = "--ember"
const shadowColorToken = "--ink"

const tokenValue = name =>
  window.getComputedStyle(document.documentElement).getPropertyValue(name).trim()

const colorWithAlpha = (color, alpha) => {
  const match = /^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec(color)
  if (!match) return color

  const [, red, green, blue] = match
  return `rgba(${parseInt(red, 16)}, ${parseInt(green, 16)}, ${parseInt(blue, 16)}, ${alpha})`
}

const configureFromTokens = () => {
  const progressColor = tokenValue(progressColorToken)
  const shadowColor = tokenValue(shadowColorToken)
  const options = {}

  if (progressColor) options.barColors = {0: progressColor}
  if (shadowColor) options.shadowColor = colorWithAlpha(shadowColor, 0.3)

  topbar.config(options)
}

export const installNavigationProgress = () => {
  window.addEventListener("phx:page-loading-start", () => {
    configureFromTokens()
    topbar.show(progressDelay)
  })
  window.addEventListener("phx:page-loading-stop", () => topbar.hide())
}
