//! Owns packet-bound conservative execution-unit reservations, not prices, exact billing,
//! monetary conversion, host grant approval/refunds or a lifetime/lost-attempt ledger.

use crate::{
    format::Id32,
    preparation::{InputBinding, ValidatedInput},
};
use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnitError {
    WrongInput,
    Exhausted,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WrongInput => "execution-unit grant belongs to another prepared packet",
            Self::Exhausted => "activation or remaining Episode unit capacity exhausted",
        })
    }
}

impl core::error::Error for UnitError {}

/// One checked packet's host-profile unit capacities. A reservation is an authorized
/// upper-bound debit, NOT an exact cost report: null/unknown billing never becomes zero.
/// Quotes/units/fresh grant provenance must come from the trusted profile, not model args.
/// Root Usage cannot reconstruct global remaining units or refund a lost paid attempt.
///
/// Private non-Clone mutable state starts once per admitted Activation and cannot top up,
/// reset or refund. The enclosing runtime must retain this same ledger across inference.
/// Count and unit guards compose before effects; zero available units pauses/fails even
/// a zero quote, while a free operation with positive remaining units is still count-bounded.
#[derive(Debug, Eq, PartialEq)]
pub struct UnitBudget {
    binding: InputBinding,
    admission_id: Id32,
    activation: u64,
    episode: u64,
    reserved: u64,
}

impl UnitBudget {
    pub fn from_input(input: &ValidatedInput<'_>) -> Self {
        let grant = input.input().budget_admission();
        Self {
            binding: input.binding(),
            admission_id: grant.admission_id,
            activation: grant.activation_units,
            episode: grant.remaining_episode_units,
            reserved: 0,
        }
    }

    pub fn binding(&self) -> InputBinding {
        self.binding
    }

    pub fn admission_id(&self) -> Id32 {
        self.admission_id
    }

    pub fn reserved_units(&self) -> u64 {
        self.reserved
    }

    pub fn remaining_activation_units(&self) -> u64 {
        self.activation - self.reserved
    }

    pub fn remaining_episode_units(&self) -> u64 {
        self.episode - self.reserved
    }

    pub(crate) fn check(&self, binding: InputBinding, quote: u64) -> Result<(), UnitError> {
        if self.binding != binding {
            return Err(UnitError::WrongInput);
        }
        let available = self
            .remaining_activation_units()
            .min(self.remaining_episode_units());
        if available == 0 || quote > available {
            return Err(UnitError::Exhausted);
        }
        Ok(())
    }

    pub(crate) fn commit(&mut self, quote: u64) {
        // Only a successful immutable check followed by count/nonce admission reaches this.
        // No external callback or ledger mutation occurs between check and commit.
        self.reserved = self
            .reserved
            .checked_add(quote)
            .expect("prechecked unit reservation fits u64 capacities");
    }
}
