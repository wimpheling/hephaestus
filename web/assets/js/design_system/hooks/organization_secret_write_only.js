// The server emits this only after capturing the submit parameters. No submit
// listener can clear a field before LiveView serializes its request.
export const OrganizationSecretWriteOnly = {
  mounted() {
    this.clearPlaintext = () => {
      const input = this.el.querySelector('[name="secret[value]"]')
      if (!input) return

      input.value = ""
      input.defaultValue = ""
      input.removeAttribute("value")
    }
    this.clearEvent = this.handleEvent("organization-secret-clear", () => this.clearPlaintext())
  },

  destroyed() {
    this.clearPlaintext()
    this.removeHandleEvent(this.clearEvent)
  },
}
