export const UnsafeMarkup = {
  mounted() {
    this.el.innerHTML = "<strong>Injected application markup</strong>"
    this.el.insertAdjacentHTML("beforeend", "<button>Continue</button>")
    this.el.outerHTML = "<main>replacement</main>"
    document.createElement("dialog")
    new DOMParser().parseFromString("<p>parsed</p>", "text/html")
    document.createRange().createContextualFragment("<p>fragment</p>")
    document.write("<p>written</p>")
  },
}
