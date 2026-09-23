const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/**
 * Reads the generic authenticated installed-UI target projection.
 *
 * The host verifies the HttpOnly child cookie and returns only the repository
 * target selected by the installation. This adapter never reads that cookie,
 * accepts a URL-selected repository, or sends an Authorization header.
 */
export async function loadUiContext(fetchImpl = globalThis.fetch) {
  const response = await fetchImpl("/_heph/ui-context", {
    credentials: "include",
    headers: { Accept: "application/json" },
  });
  if (!response.ok) throw new Error("the authenticated UI context is unavailable");
  const context = await response.json();
  if (
    !context ||
    Object.keys(context).length !== 1 ||
    typeof context.repository_id !== "string" ||
    !UUID.test(context.repository_id)
  ) {
    throw new Error("the authenticated UI context has an invalid repository target");
  }
  return { repositoryId: context.repository_id };
}
