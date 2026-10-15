//! Owns immutable initial context and appended model-data trajectory, not ready inference.
//! Declared descriptors are data, not a registered/budget-enabled ToolSet or host admission.

use super::{encoding::*, HistoryProjection, ProjectionError};
use crate::{
    definition::{ToolControl, ToolDescriptorView},
    format::{DecodeLimits, ModelValue, RecordEncoder, ValueView},
    graph::RestoredAgent,
    input::ActivationInput,
    limits::Limits,
    preparation::{check_input, InputCompatibilityError},
    receipt::ReceiptView,
};

/// Immutable six-section prefix: policy/mandate, declared tools, machine input,
/// completed history, current chain and initial checkpoint Memory. New RAM content
/// appends in a seventh section; patches cannot rewrite this captured prefix.
///
/// This is complete context DATA, not an Activation/ModelRequest or approved inference.
/// Incoming-input wire caps and phase/Receipt consistency are checked; the snapshot
/// was bounded at restoration. Host source/freshness,
/// registration/code/schema agreement, enabled tool selection, budgets/units and
/// request/reply binding remain prepare/activation/host duties. Provider JSON/HTTP
/// limits remain separate from this canonical normalized-CBOR projection accounting.
#[derive(Debug, Eq, PartialEq)]
pub struct InitialProjection {
    prefix: ModelValue,
    bounds: DecodeLimits,
}

impl InitialProjection {
    pub fn build(
        snapshot: &RestoredAgent,
        input: &ActivationInput,
        caller: &Limits,
    ) -> Result<Self, ProjectionError> {
        let definition = snapshot.definition();
        let limits = definition.limits();
        check_input(snapshot, input, caller).map_err(|error| match error {
            InputCompatibilityError::Input(error) => ProjectionError::Input(error),
            InputCompatibilityError::WrongPhase => ProjectionError::WrongPhase,
            InputCompatibilityError::ReceiptMismatch => ProjectionError::ReceiptMismatch,
        })?;
        let bounds = context_bounds(limits, caller);
        let mut writer = RecordEncoder::new(&bounds)?;
        list(&mut writer, 6, 1, &bounds)?;
        map(&mut writer, 3, 2, &bounds)?;
        writer.text("kind")?;
        writer.text("policy")?;
        writer.text("policy")?;
        writer.text(definition.policy())?;
        writer.text("mandate")?;
        writer.text(definition.mandate())?;
        map(&mut writer, 2, 2, &bounds)?;
        writer.text("kind")?;
        writer.text("tools")?;
        writer.text("tools")?;
        list(&mut writer, definition.tools().len() as u64, 3, &bounds)?;
        for descriptor in definition.tools() {
            tool(&mut writer, descriptor, &bounds)?;
        }
        machine(&mut writer, input, &bounds)?;
        history_sections(&mut writer, HistoryProjection::new(snapshot), &bounds)?;
        map(&mut writer, 2, 2, &bounds)?;
        writer.text("kind")?;
        writer.text("memory")?;
        writer.text("memory")?;
        memory(&mut writer, snapshot.memory(), 3, &bounds)?;
        Ok(Self {
            prefix: ModelValue::from_encoded(writer.finish(), &bounds)?,
            bounds,
        })
    }

    /// Exact immutable initial data; this alone has no RAM trajectory section yet.
    pub fn as_bytes(&self) -> &[u8] {
        self.prefix.as_bytes()
    }

    /// Compose all seven ordered sections and bound their combined canonical byte size,
    /// including declared descriptors, all retained Episodes and trajectory content.
    /// Trajectory must be a canonical model List; its entries are data, not accepted
    /// tool results. This method neither changes memory nor validates/executes calls.
    pub fn context(&self, trajectory: &ModelValue) -> Result<ModelValue, ProjectionError> {
        let value = ValueView::preflighted(trajectory.as_bytes());
        value
            .list()
            .map_err(|_| ProjectionError::TrajectoryNotList)?;
        let mut writer = RecordEncoder::new(&self.bounds)?;
        list(&mut writer, 7, 1, &self.bounds)?;
        for section in ValueView::preflighted(self.prefix.as_bytes())
            .list()
            .expect("captured prefix List")
        {
            writer.encoded(section.expect("validated prefix section").encoded())?;
        }
        map(&mut writer, 2, 2, &self.bounds)?;
        writer.text("kind")?;
        writer.text("ram_trajectory")?;
        writer.text("entries")?;
        model(&mut writer, trajectory.as_bytes(), 3, &self.bounds)?;
        Ok(ModelValue::from_encoded(writer.finish(), &self.bounds)?)
    }
}

fn tool(
    writer: &mut RecordEncoder,
    value: ToolDescriptorView<'_>,
    bounds: &DecodeLimits,
) -> Result<(), crate::format::CodecError> {
    map(writer, 6, 4, bounds)?;
    writer.text("id")?;
    writer.text(value.id)?;
    writer.text("control")?;
    writer.text(match value.control {
        ToolControl::Continue => "continue",
        ToolControl::Yield => "yield",
        ToolControl::Finish => "finish",
    })?;
    writer.text("version")?;
    number(writer, value.version)?;
    writer.text("description")?;
    writer.text(value.description)?;
    writer.text("arguments_schema")?;
    model(writer, value.arguments_schema, 5, bounds)?;
    writer.text("authority_binding")?;
    if let Some(binding) = value.authority_binding {
        id(writer, binding)?;
    } else {
        writer.null()?;
    }
    Ok(())
}

fn machine(
    writer: &mut RecordEncoder,
    value: &ActivationInput,
    bounds: &DecodeLimits,
) -> Result<(), crate::format::CodecError> {
    map(writer, 7, 2, bounds)?;
    writer.text("kind")?;
    writer.text("machine_input")?;
    writer.text("cause")?;
    writer.text(value.cause().as_str())?;
    writer.text("receipt")?;
    if let Some(bytes) = value.receipt_bytes() {
        receipt(writer, ReceiptView::from_validated(bytes), 3, bounds)?;
    } else {
        writer.null()?;
    }
    writer.text("event_id")?;
    id(writer, value.event_id())?;
    writer.text("observation")?;
    model(writer, value.observation_bytes(), 3, bounds)?;
    writer.text("observed_at")?;
    position(writer, value.observed_at(), 3, bounds)?;
    writer.text("budget_admission")?;
    let grant = value.budget_admission();
    map(writer, 5, 3, bounds)?;
    writer.text("admission_id")?;
    id(writer, grant.admission_id)?;
    writer.text("activation_units")?;
    number(writer, grant.activation_units)?;
    writer.text("remaining_actions")?;
    number(writer, grant.remaining_actions)?;
    writer.text("remaining_model_calls")?;
    number(writer, grant.remaining_model_calls)?;
    writer.text("remaining_episode_units")?;
    number(writer, grant.remaining_episode_units)?;
    Ok(())
}
