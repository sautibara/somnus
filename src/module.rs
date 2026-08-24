use std::sync::Arc;

use auditeur::{
    component::{
        IdentifiedComponent, IdentifiedType, RegistryOf,
        registry::{ErasedLoader, Loaded, Loader, Registries, Registry},
    },
    prelude::*,
};

pub trait Module:
    IdentifiedType + Create + auditeur::any::AnyConst + Send + Sync + 'static
{
    fn parts() -> ModuleParts;
}

#[derive(Default)]
pub struct ModuleParts {}

#[identified("somnus:modules")]
pub struct Modules;

#[derive(Clone)]
pub(crate) struct SharedModule {
    module: Arc<dyn auditeur::any::Any + Send + Sync>,
}

#[derive(auditeur::Any)]
struct SharedModuleInner<M: Module> {
    rwlock: tokio::sync::RwLock<M>,
}

impl SharedModule {
    fn new<M: Module>(module: M) -> Self {
        Self {
            module: Arc::new(SharedModuleInner::<M> {
                rwlock: tokio::sync::RwLock::new(module),
            }),
        }
    }

    pub(crate) async fn read<M: Module>(&self) -> tokio::sync::RwLockReadGuard<'_, M> {
        let module: &SharedModuleInner<M> = self
            .module
            .downcast_ref()
            .expect("downcast should not fail");
        module.rwlock.read().await
    }

    pub(crate) async fn write<M: Module>(&self) -> tokio::sync::RwLockWriteGuard<'_, M> {
        let module: &SharedModuleInner<M> = self
            .module
            .downcast_ref()
            .expect("downcast should not fail");
        module.rwlock.write().await
    }
}

impl RegistryOf for Modules {
    type Registry = Registries;
}

impl Registry for Modules {
    type Loader = ErasedLoader<ModuleLoaded>;
    type Loaded = ModuleLoaded;
}

#[doc(hidden)]
pub struct ModuleLoader<M: Module> {
    input: M::ExternalInput,
}

impl<M: Module> Loader<ErasedLoader<ModuleLoaded>> for ModuleLoader<M> {
    type Loaded = ModuleLoaded;

    async fn load(&self, app: &mut App) -> Result<Self::Loaded, auditeur::Error> {
        let module = M::create(&self.input, app).await?;
        let module = SharedModule::new(module);
        Ok(ModuleLoaded { module })
    }
}

impl<M: Module> IdentifiedComponent for ModuleLoader<M> {
    fn component_id_dyn(&self) -> ComponentId {
        ComponentId::inn::<Modules>(M::id())
    }
}

#[doc(hidden)]
#[derive(auditeur::Any)]
pub struct ModuleLoaded {
    pub(crate) module: SharedModule,
}

impl Loaded for ModuleLoaded {}
