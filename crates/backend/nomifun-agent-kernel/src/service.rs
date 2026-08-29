use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::sync::Arc;

use nomifun_agent_contracts::{
    PackageRef, PluginMountId, ServiceHandleDescriptor, ServiceKeyId, ServiceKeyRef, VersionString,
};

use crate::KernelError;

pub struct ServiceKey<T: ?Sized + Send + Sync + 'static> {
    reference: ServiceKeyRef,
    marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized + Send + Sync + 'static> Clone for ServiceKey<T> {
    fn clone(&self) -> Self {
        Self {
            reference: self.reference.clone(),
            marker: PhantomData,
        }
    }
}

impl<T: ?Sized + Send + Sync + 'static> ServiceKey<T> {
    pub fn new(id: impl Into<ServiceKeyId>, version: impl Into<VersionString>) -> Self {
        Self {
            reference: ServiceKeyRef {
                id: id.into(),
                version: version.into(),
            },
            marker: PhantomData,
        }
    }

    pub fn from_ref(reference: ServiceKeyRef) -> Self {
        Self {
            reference,
            marker: PhantomData,
        }
    }

    pub fn reference(&self) -> &ServiceKeyRef {
        &self.reference
    }
}

#[derive(Clone)]
pub(crate) struct ErasedService {
    descriptor: ServiceHandleDescriptor,
    value: Arc<dyn Any + Send + Sync>,
}

#[derive(Clone, Default)]
pub struct ServiceExports {
    values: BTreeMap<ServiceKeyRef, Arc<dyn Any + Send + Sync>>,
}

impl ServiceExports {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provide<T>(
        &mut self,
        key: &ServiceKey<T>,
        service: Arc<T>,
    ) -> Result<(), KernelError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        if self
            .values
            .insert(key.reference.clone(), Arc::new(service))
            .is_some()
        {
            return Err(KernelError::DuplicateServiceProvider {
                service_id: key.reference.id.clone(),
            });
        }
        Ok(())
    }

    pub fn provided_refs(&self) -> BTreeSet<ServiceKeyRef> {
        self.values.keys().cloned().collect()
    }

    pub(crate) fn values(
        &self,
    ) -> impl Iterator<Item = (&ServiceKeyRef, &Arc<dyn Any + Send + Sync>)> {
        self.values.iter()
    }
}

#[derive(Clone, Default)]
pub struct DeclaredServiceView {
    services: BTreeMap<ServiceKeyRef, ErasedService>,
}

impl DeclaredServiceView {
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    pub fn descriptors(&self) -> Vec<ServiceHandleDescriptor> {
        self.services
            .values()
            .map(|service| service.descriptor.clone())
            .collect()
    }

    pub fn require<T>(&self, key: &ServiceKey<T>) -> Result<Arc<T>, KernelError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let Some(service) = self.services.get(key.reference()) else {
            return Err(KernelError::MissingService {
                mount_id: PluginMountId::from("declared-service-view"),
                service_id: key.reference.id.clone(),
                version: key.reference.version.clone(),
            });
        };
        service
            .value
            .downcast_ref::<Arc<T>>()
            .cloned()
            .ok_or_else(|| KernelError::ServiceTypeMismatch {
                service_id: key.reference.id.clone(),
            })
    }

    pub(crate) fn from_bindings(
        required: &[ServiceHandleDescriptor],
        services: &BTreeMap<ServiceKeyRef, ErasedService>,
    ) -> Result<Self, KernelError> {
        let mut declared = BTreeMap::new();
        for descriptor in required {
            let Some(service) = services.get(&descriptor.service) else {
                return Err(KernelError::MissingService {
                    mount_id: descriptor.provider_mount_id.clone(),
                    service_id: descriptor.service.id.clone(),
                    version: descriptor.service.version.clone(),
                });
            };
            declared.insert(descriptor.service.clone(), service.clone());
        }
        Ok(Self { services: declared })
    }
}

pub(crate) fn build_service_bindings(
    providers: impl IntoIterator<
        Item = (
            PackageRef,
            PluginMountId,
            ServiceExports,
        ),
    >,
) -> Result<BTreeMap<ServiceKeyRef, ErasedService>, KernelError> {
    let mut services = BTreeMap::new();
    let mut ids = BTreeMap::<ServiceKeyId, VersionString>::new();
    for (package, mount_id, exports) in providers {
        for (service_ref, value) in exports.values() {
            if let Some(existing) = ids.get(&service_ref.id) {
                return if existing == &service_ref.version {
                    Err(KernelError::DuplicateServiceProvider {
                        service_id: service_ref.id.clone(),
                    })
                } else {
                    Err(KernelError::DuplicateServiceProvider {
                        service_id: service_ref.id.clone(),
                    })
                };
            }
            ids.insert(service_ref.id.clone(), service_ref.version.clone());
            services.insert(
                service_ref.clone(),
                ErasedService {
                    descriptor: ServiceHandleDescriptor {
                        service: service_ref.clone(),
                        provider_package: package.clone(),
                        provider_mount_id: mount_id.clone(),
                    },
                    value: Arc::clone(value),
                },
            );
        }
    }
    Ok(services)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    trait EchoPort: Send + Sync {
        fn echo(&self, value: &str) -> String;
    }

    struct EchoService;

    impl EchoPort for EchoService {
        fn echo(&self, value: &str) -> String {
            format!("typed:{value}")
        }
    }

    #[test]
    fn typed_service_view_returns_only_the_declared_exact_type() {
        let key = ServiceKey::<dyn EchoPort>::new("service.sample.echo", "1.0.0");
        let mut exports = ServiceExports::new();
        exports
            .provide(&key, Arc::new(EchoService) as Arc<dyn EchoPort>)
            .unwrap();
        let package = PackageRef {
            id: nomifun_agent_contracts::PackageId::from("sample.provider"),
            version: VersionString::from("1.0.0"),
        };
        let mount = PluginMountId::from("sample-provider");
        let services =
            build_service_bindings([(package.clone(), mount.clone(), exports)]).unwrap();
        let descriptor = services
            .values()
            .next()
            .unwrap()
            .descriptor
            .clone();
        let view = DeclaredServiceView::from_bindings(&[descriptor], &services).unwrap();
        assert_eq!(view.require(&key).unwrap().echo("ok"), "typed:ok");

        let wrong = ServiceKey::<String>::new("service.sample.echo", "1.0.0");
        assert!(matches!(
            view.require(&wrong),
            Err(KernelError::ServiceTypeMismatch { .. })
        ));
    }
}
