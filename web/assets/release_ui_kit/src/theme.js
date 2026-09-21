/* Optional release UI theme and parent-ready helper, versioned with the kit. */
(() => {
  function exactHttpsOrigin(values) {
    if (values.length !== 1 || !/^https:\/\/[A-Za-z0-9.-]+(?::[1-9][0-9]{0,4})?$/.test(values[0])) {
      return null;
    }
    try {
      const parsed = new URL(values[0]);
      if (
        parsed.protocol !== "https:" ||
        parsed.origin !== values[0] ||
        parsed.username ||
        parsed.password ||
        parsed.pathname !== "/" ||
        parsed.search ||
        parsed.hash ||
        parsed.hostname.endsWith(".")
      ) {
        return null;
      }
      return parsed.origin;
    } catch {
      return null;
    }
  }

  const validThemes = new Set(["light", "dark"]);
  const params = new URLSearchParams(window.location.search);
  const themeValues = params.getAll("heph_theme");
  const theme = themeValues.length === 1 && validThemes.has(themeValues[0])
    ? themeValues[0]
    : null;

  if (theme) {
    document.documentElement.dataset.theme = theme;
  } else {
    delete document.documentElement.dataset.theme;
  }

  const parentOrigin = exactHttpsOrigin(params.getAll("heph_theme_origin"));
  if (!parentOrigin) {
    return;
  }

  const applyTheme = value => {
    if (validThemes.has(value)) {
      document.documentElement.dataset.theme = value;
    }
  };

  window.addEventListener("message", event => {
    if (event.source !== window.parent || event.origin !== parentOrigin) {
      return;
    }
    const message = event.data;
    if (!message || typeof message !== "object") {
      return;
    }
    const keys = Object.keys(message);
    if (keys.length !== 2 || message.type !== "heph-ui-theme" || !validThemes.has(message.theme)) {
      return;
    }
    applyTheme(message.theme);
  });

  if (window.parent !== window) {
    const announceReady = () => {
      window.parent.postMessage({type: "heph-ui-ready"}, parentOrigin);
    };
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", announceReady, {once: true});
    } else {
      announceReady();
    }
  }
})();
