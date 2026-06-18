use std::collections::{BTreeMap, BTreeSet};

use crate::{
    engine::model::ResolvedRootModule,
    model::protofetch::{
        resolved::{ResolvedDependency, ResolvedModule},
        ModuleName, Rules,
    },
};

impl From<ResolvedRootModule> for ResolvedModule {
    fn from(root: ResolvedRootModule) -> Self {
        let mut rules = BTreeMap::<ModuleName, Vec<Rules>>::new();
        for dependency in root.dependencies.iter() {
            rules
                .entry(dependency.name.clone())
                .or_default()
                .push(dependency.rules.clone());
        }
        for module in root.modules.iter() {
            for dependency in module.dependencies.iter() {
                rules
                    .entry(dependency.name.clone())
                    .or_default()
                    .push(dependency.rules.clone());
            }
        }

        ResolvedModule {
            module_name: root.name,
            dependencies: root
                .modules
                .into_iter()
                .map(|module| ResolvedDependency {
                    name: module.name.clone(),
                    commit_hash: module.commit_hash,
                    coordinate: module.coordinate,
                    specification: module.specification,
                    dependencies: module
                        .dependencies
                        .into_iter()
                        .map(|dependency| dependency.name)
                        .collect::<BTreeSet<_>>(),
                    rules: rules.remove(&module.name).unwrap_or_default(),
                })
                .collect(),
        }
    }
}
