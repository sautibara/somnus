use std::sync::Arc;

use auditeur::{
    component::ComponentIdPath,
    prelude::{anyhow::Context, *},
};

use crate::module::{Modules, SharedModule};

#[resource("somnus:somnus")]
#[create(default)]
#[listener(on_add_in(Modules))]
#[derive(CloneIn)]
pub struct Somnus {
    state: GlobalState,

    #[create(input)]
    app: App,
}

impl OnAddIn<Modules> for Somnus {
    async fn on_add(&mut self, id: ComponentIdPath) -> anyhow::Result<()> {
        let module = (self.app.dependent_mut().with_ty(DependencyType::Removable))
            .components_mut()
            .get::<Modules, _>(id.clone(), async |loaded, _| Ok(loaded.module.clone()))
            .await
            .context("failed to initialize module")?;
        self.state.modules.insert(id, Arc::new(module));
        Ok(())
    }
}

impl Listen<auditeur::app::Event> for Somnus {
    async fn on(&mut self, message: auditeur::app::Event) -> anyhow::Result<()> {
        if let auditeur::app::Event::RequestDependencyRemoval { id } = message
            && id.dependent == Self::component_id()
            && id.dependency.registry == Modules::id()
        {
            self.state.modules.remove(&id.dependency.component);
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(crate) struct GlobalState {
    pub modules: imbl::HashMap<ComponentIdPath, Arc<Dependency<SharedModule>>>,
}
