//! Owns Definition-bound callable registration metadata, not tool execution or host approval.
//! Native callback purity/schema honesty is a trusted-code contract, not a sandbox proof.

use crate::{
    definition::{Definition, MemoryProfile, ToolControl, ToolDescriptorView},
    format::{DaslCid, ModelValue},
    input::ActivationInput,
    limits::Limits,
    memory::Memory,
};
use core::fmt;

/// Read-only inputs to a trusted pure implementation. Core owns mutable working memory
/// and command/control fields; planner output cannot select its own ID/version/authority.
#[derive(Clone, Copy)]
pub struct PlanningContext<'a> {
    pub definition: &'a Definition,
    pub input: &'a ActivationInput,
    pub memory: Option<&'a Memory>,
    pub limits: &'a Limits,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolFailure {
    InvalidArguments,
    PlanningFailed,
}
impl fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidArguments => "tool arguments failed validation",
            Self::PlanningFailed => "pure tool planning failed",
        })
    }
}
impl core::error::Error for ToolFailure {}

pub type ArgumentValidator = fn(&[u8]) -> Result<(), ToolFailure>;
pub type Planner = for<'context, 'arguments> fn(
    PlanningContext<'context>,
    &'arguments [u8],
) -> Result<ModelValue, ToolFailure>;

/// Compiled capabilities, never inferred from a model-provided tool name. Pure and Yield
/// callbacks validate application arguments then produce local model data or planned
/// Command arguments respectively. They do not sign/send. The lifecycle must enforce
/// cardinality, bounds, control, budgets and validation BEFORE invoking these functions.
#[derive(Clone, Copy)]
pub enum ToolImplementation {
    MemoryPatch,
    Pure {
        validate: ArgumentValidator,
        plan: Planner,
    },
    Yield {
        validate: ArgumentValidator,
        plan: Planner,
    },
}
impl ToolImplementation {
    pub fn control(self) -> ToolControl {
        match self {
            Self::MemoryPatch | Self::Pure { .. } => ToolControl::Continue,
            Self::Yield { .. } => ToolControl::Yield,
        }
    }
}

/// Caller-supplied compiled metadata plus implementation. Matching the Definition proves
/// declared agreement only, not that this caller/code was authorized or its validator is
/// truthful. Registration provenance and program/module binding remain host duties.
#[derive(Clone, Copy)]
pub struct ToolRegistration<'a> {
    pub descriptor: ToolDescriptorView<'a>,
    pub implementation: ToolImplementation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSetError {
    CountLimit,
    DuplicateRegistration,
    UnlistedRegistration,
    MissingRegistration,
    VersionMismatch,
    ControlMismatch,
    DescriptionMismatch,
    SchemaMismatch,
    AuthorityMismatch,
    ImplementationControlMismatch,
    MemoryDisabled,
    DefinitionMismatch,
}
impl fmt::Display for ToolSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CountLimit => "tool registrations exceed caller/Definition count limit",
            Self::DuplicateRegistration => "tool registrations repeat an ID",
            Self::UnlistedRegistration => "registration is outside the Definition whitelist",
            Self::MissingRegistration => "Definition tool has no implementation registration",
            Self::VersionMismatch => "registered tool version differs from Definition",
            Self::ControlMismatch => "registered tool control differs from Definition",
            Self::DescriptionMismatch => "registered tool description differs from Definition",
            Self::SchemaMismatch => "registered tool schema bytes differ from Definition",
            Self::AuthorityMismatch => "registered tool authority differs from Definition",
            Self::ImplementationControlMismatch => "implementation does not match declared control",
            Self::MemoryDisabled => {
                "memory-disabled profile cannot bind a memory-patch implementation"
            }
            Self::DefinitionMismatch => "ToolSet was bound to another Definition",
        })
    }
}
impl core::error::Error for ToolSetError {}

/// An exact Definition/registration bijection, borrowed without payload/metadata allocation.
/// Definition-order views require the same exact Definition CID. The set retains only
/// that CID and caller registrations, not a reference into the future owned snapshot.
/// This is not a budget-enabled ToolSet or an approved executable program. No callbacks
/// are invoked during binding/lookup. Extra/missing/mismatched tools fail atomically;
/// there is no generic fallback and no privilege gained through names or patches.
/// Copying this immutable view retains the exact CID and borrowed registrations, with
/// no rebinding, allocation, callbacks or authority. It never extends their lifetime.
///
/// ```compile_fail
/// use raiv::{definition::Definition, limits::Limits, tools::{ToolRegistration, ToolSet}};
/// fn cannot_outlive_registrations<'a>(definition: &Definition) -> ToolSet<'a> {
///     let registrations: [ToolRegistration<'_>; 0] = [];
///     let bound = ToolSet::bind(definition, &registrations, &Limits::default()).unwrap();
///     bound.clone()
/// }
/// ```
#[derive(Clone, Copy)]
pub struct ToolSet<'a> {
    definition: DaslCid,
    registrations: &'a [ToolRegistration<'a>],
}
impl<'a> ToolSet<'a> {
    pub fn bind(
        definition: &Definition,
        registrations: &'a [ToolRegistration<'a>],
        caller: &Limits,
    ) -> Result<Self, ToolSetError> {
        let ceiling = caller
            .max_collection_entries
            .min(definition.limits().max_collection_entries);
        if registrations.len() as u64 > ceiling || definition.tools().len() as u64 > ceiling {
            return Err(ToolSetError::CountLimit);
        }
        for (index, registration) in registrations.iter().enumerate() {
            let actual = registration.descriptor;
            if registrations[..index]
                .iter()
                .any(|tool| tool.descriptor.id == actual.id)
            {
                return Err(ToolSetError::DuplicateRegistration);
            }
            let expected = definition
                .tools()
                .find(|tool| tool.id == actual.id)
                .ok_or(ToolSetError::UnlistedRegistration)?;
            if actual.version != expected.version {
                return Err(ToolSetError::VersionMismatch);
            }
            if actual.control != expected.control {
                return Err(ToolSetError::ControlMismatch);
            }
            if actual.description != expected.description {
                return Err(ToolSetError::DescriptionMismatch);
            }
            if actual.arguments_schema != expected.arguments_schema {
                return Err(ToolSetError::SchemaMismatch);
            }
            if actual.authority_binding != expected.authority_binding {
                return Err(ToolSetError::AuthorityMismatch);
            }
            if registration.implementation.control() != expected.control {
                return Err(ToolSetError::ImplementationControlMismatch);
            }
            if definition.memory_profile() == MemoryProfile::None
                && matches!(registration.implementation, ToolImplementation::MemoryPatch)
            {
                return Err(ToolSetError::MemoryDisabled);
            }
        }
        if definition.tools().len() != registrations.len() {
            return Err(ToolSetError::MissingRegistration);
        }
        Ok(Self {
            definition: definition.cid(),
            registrations,
        })
    }
    pub fn definition_cid(&self) -> DaslCid {
        self.definition
    }
    /// Lookup only a whitelisted/bound ID, borrowing compiled registration metadata.
    /// Unknown names do not dispatch to a fallback or invoke callbacks.
    pub fn lookup(&self, id: &str) -> Option<BoundTool<'a>> {
        let registered = self
            .registrations
            .iter()
            .find(|tool| tool.descriptor.id == id)?;
        Some(BoundTool {
            descriptor: registered.descriptor,
            implementation: registered.implementation,
        })
    }
    /// Borrow descriptor views in Definition order after checking its exact CID.
    pub fn tools<'view>(
        &'view self,
        definition: &'view Definition,
    ) -> Result<impl Iterator<Item = BoundTool<'view>> + 'view, ToolSetError> {
        if definition.cid() != self.definition {
            return Err(ToolSetError::DefinitionMismatch);
        }
        Ok(definition.tools().map(|descriptor| BoundTool {
            descriptor,
            implementation: self
                .lookup(descriptor.id)
                .expect("bound Definition tool")
                .implementation,
        }))
    }
}

/// Read-only metadata and a supplied capability. Access does not debit a budget, invoke
/// a validator/planner or grant dispatch. Application/lifecycle code must use its normal
/// admission/control path, not treat this handle as a ready inference/execution permit.
#[derive(Clone, Copy)]
pub struct BoundTool<'a> {
    descriptor: ToolDescriptorView<'a>,
    implementation: ToolImplementation,
}
impl<'a> BoundTool<'a> {
    pub fn descriptor(self) -> ToolDescriptorView<'a> {
        self.descriptor
    }
    pub fn implementation(self) -> ToolImplementation {
        self.implementation
    }
}

#[cfg(test)]
mod tests;
