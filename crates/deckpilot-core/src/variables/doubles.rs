//! In-memory implementations of the resolver traits, for tests of the core,
//! the modules and the agent. They follow the same rules as the agent's
//! resolver must: explicit context, precedence, profile isolation, and no
//! value for what exists only at run time.

use std::collections::{BTreeMap, BTreeSet};

use deckpilot_device::DeviceId;

use super::resolution::{
    DeviceContext, MissingReason, ProjectResolver, ProjectSource, Resolution, ResolutionContext,
    ResolutionPhase, ValueSource, VariableResolver, select_value,
};
use super::{
    ScopeKey, SystemVariable, VariableDefinition, VariableKey, VariableRef, VariableValue,
};
use crate::graph::NodeId;
use crate::{Os, Profile, ProfileId, ProjectId};

/// Shared variables, one profile for system variables, and node outputs of
/// one run.
#[derive(Debug, Clone)]
pub struct MemoryVariables {
    profile: Profile,
    os: Os,
    definitions: BTreeMap<VariableKey, VariableDefinition>,
    values: BTreeMap<VariableKey, Vec<(ScopeKey, VariableValue)>>,
    outputs: BTreeMap<(NodeId, String), String>,
    denied: BTreeSet<VariableKey>,
}

impl MemoryVariables {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            os: Os::Linux,
            definitions: BTreeMap::new(),
            values: BTreeMap::new(),
            outputs: BTreeMap::new(),
            denied: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn define(mut self, definition: VariableDefinition) -> Self {
        self.definitions.insert(definition.key.clone(), definition);
        self
    }

    /// Sets a value; replaces an earlier value at the same scope key.
    ///
    /// # Panics
    ///
    /// If the variable is not defined or the value does not fit it: a
    /// mistake in the test.
    #[must_use]
    pub fn set(mut self, key: &VariableKey, scope: ScopeKey, value: VariableValue) -> Self {
        let definition = self
            .definitions
            .get(key)
            .expect("define the variable first");
        definition
            .check_value(&scope, &value)
            .expect("the value fits the definition");
        let values = self.values.entry(key.clone()).or_default();
        values.retain(|(existing, _)| *existing != scope);
        values.push((scope, value));
        self
    }

    /// An output of node `node` in the run.
    #[must_use]
    pub fn output(mut self, node: NodeId, key: &str, value: &str) -> Self {
        self.outputs
            .insert((node, key.to_owned()), value.to_owned());
        self
    }

    /// Reading `key` is not permitted.
    #[must_use]
    pub fn deny(mut self, key: VariableKey) -> Self {
        self.denied.insert(key);
        self
    }

    #[must_use]
    pub fn on(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    pub fn definitions(&self) -> impl Iterator<Item = &VariableDefinition> {
        self.definitions.values()
    }

    fn shared(&self, key: &VariableKey, context: &ResolutionContext) -> Resolution {
        if self.denied.contains(key) {
            return Resolution::PermissionDenied;
        }
        let Some(definition) = self.definitions.get(key) else {
            return Resolution::Missing(MissingReason::UnknownVariable);
        };
        let values = self.values.get(key).into_iter().flatten();
        let Some((value, source)) = select_value(
            definition,
            values.map(|(scope, value)| (scope, value)),
            context,
        ) else {
            return Resolution::Missing(MissingReason::NoValue);
        };
        let foreign = match value {
            VariableValue::ProjectId(id) => self.profile.project(*id).is_none(),
            VariableValue::ProfileId(id) => *id != self.profile.id,
            _ => false,
        };
        if foreign {
            return Resolution::Missing(MissingReason::OtherProfile);
        }
        Resolution::Value {
            value: value.clone(),
            source,
            sensitive: definition.sensitive,
        }
    }

    fn system(&self, variable: SystemVariable, context: &ResolutionContext) -> Resolution {
        let preview = context.phase == ResolutionPhase::Preview;
        let value = |value: VariableValue| Resolution::Value {
            value,
            source: ValueSource::System,
            sensitive: false,
        };
        match variable {
            SystemVariable::ProjectCurrentId
            | SystemVariable::ProjectCurrentKey
            | SystemVariable::ProjectCurrentName => {
                // A run's project is known only once it runs; the active
                // project is not a stand-in for it.
                if preview && context.project_source == ProjectSource::Execution {
                    return Resolution::UnavailableUntilRuntime;
                }
                let Some(id) = context.project else {
                    return Resolution::Missing(MissingReason::NoProject);
                };
                let Some(project) = self.profile.project(id) else {
                    return Resolution::Missing(MissingReason::OtherProfile);
                };
                value(match variable {
                    SystemVariable::ProjectCurrentId => VariableValue::ProjectId(project.id),
                    SystemVariable::ProjectCurrentKey => {
                        VariableValue::String(project.key.to_string())
                    }
                    _ => VariableValue::String(project.name.clone()),
                })
            }
            SystemVariable::ProfileCurrentId => value(VariableValue::ProfileId(self.profile.id)),
            SystemVariable::ProfileCurrentName => {
                value(VariableValue::String(self.profile.name.clone()))
            }
            SystemVariable::DeviceCurrentId => match &context.device {
                Some(device) => value(VariableValue::String(device.to_string())),
                None => Resolution::Missing(MissingReason::NoDevice),
            },
            SystemVariable::Platform => value(VariableValue::String(
                match self.os {
                    Os::Linux => "linux",
                    Os::MacOs => "macOs",
                    Os::Windows => "windows",
                    Os::Other => "other",
                }
                .to_owned(),
            )),
            SystemVariable::WorkflowId => match context.workflow {
                Some(workflow) => value(VariableValue::String(workflow.to_string())),
                None => Resolution::Missing(MissingReason::NoWorkflow),
            },
            SystemVariable::WorkflowExecutionId => match context.execution {
                _ if preview => Resolution::UnavailableUntilRuntime,
                Some(execution) => value(VariableValue::String(execution.to_string())),
                None => Resolution::Missing(MissingReason::NoExecution),
            },
            // There is no work session outside the agent.
            SystemVariable::WorkSessionId
            | SystemVariable::WorkSessionState
            | SystemVariable::WorkSessionStartedAt
            | SystemVariable::WorkSessionElapsedSeconds
            | SystemVariable::WorkSessionProjectId
            | SystemVariable::WorkSessionCanFinish => Resolution::Missing(MissingReason::NoValue),
        }
    }
}

impl VariableResolver for MemoryVariables {
    fn resolve(&self, reference: &VariableRef, context: &ResolutionContext) -> Resolution {
        if context.profile != self.profile.id {
            return Resolution::Missing(MissingReason::OtherProfile);
        }
        match reference {
            VariableRef::Step(_) if context.phase == ResolutionPhase::Preview => {
                Resolution::UnavailableUntilRuntime
            }
            VariableRef::Step(step) => self
                .outputs
                .get(&(step.node.clone(), step.output.clone()))
                .map_or(Resolution::Missing(MissingReason::NoOutput), |text| {
                    Resolution::Value {
                        value: VariableValue::String(text.clone()),
                        source: ValueSource::StepOutput {
                            node: step.node.clone(),
                        },
                        sensitive: false,
                    }
                }),
            VariableRef::Shared(key) => self.shared(key, context),
            VariableRef::System(variable) => self.system(*variable, context),
        }
    }
}

/// The application's active project per profile.
#[derive(Debug, Clone, Default)]
pub struct FixedProjects(pub BTreeMap<ProfileId, ProjectId>);

impl ProjectResolver for FixedProjects {
    fn active_project(&self, profile: ProfileId) -> Option<ProjectId> {
        self.0.get(&profile).copied()
    }
}

/// A fixed device, or none.
#[derive(Debug, Clone, Default)]
pub struct FixedDevice(pub Option<DeviceId>);

impl DeviceContext for FixedDevice {
    fn current_device(&self) -> Option<DeviceId> {
        self.0.clone()
    }
}
