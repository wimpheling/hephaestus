//! Same Ready/close lock admits only the installed original recipe service.
use super::{AppError, Arc, ManagedBackendNode, Phase, Registry};
use crate::managed_backend::recipe::{ManagedRecipe, Request, Ticket};
impl Registry {
    pub fn install_recipe(
        &self,
        node: &ManagedBackendNode,
        recipe: Arc<ManagedRecipe>,
    ) -> Result<(), AppError> {
        let state = self.state();
        if state.poisoned
            || !state
                .node
                .as_ref()
                .is_some_and(|original| std::ptr::eq(original.as_ref(), node))
        {
            return Err(AppError::Readiness(
                "managed recipe original node differs".into(),
            ));
        }
        if self.control.is_closing() {
            recipe.close();
        }
        node.recipes
            .set(recipe)
            .map_err(|_| AppError::Readiness("managed recipe original already installed".into()))?;
        drop(state);
        Ok(())
    }
    pub fn admit_recipe(
        &self,
        recipe: &Arc<ManagedRecipe>,
        request: Request,
    ) -> Result<Ticket, AppError> {
        let state = self.state();
        if state.poisoned
            || self.control.is_closing()
            || state.status.held_reason.is_some()
            || state.status.phase != Phase::Ready
            || !state.node.as_ref().is_some_and(|node| {
                node.services.healthy()
                    && node
                        .recipes
                        .get()
                        .is_some_and(|original| Arc::ptr_eq(original, recipe))
            })
        {
            return Err(AppError::Readiness(
                "managed recipe is not Ready, or admission is closing".into(),
            ));
        }
        let ticket = recipe.submit(request)?;
        drop(state);
        Ok(ticket)
    }
}
