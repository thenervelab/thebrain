// # Copyright 2024 The Nerve Lab
// This file is part of Hippius.

// Hippius is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Hippius is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Hippius.  If not, see <http://www.gnu.org/licenses/>.

//! Environmental-aware externalities for EVM tracing in Wasm runtime. This enables
//! capturing the - potentially large - trace output data in the host and keep
//! a low memory footprint in `--execution=wasm`.
//!
//! - The original trace Runtime Api call is wrapped `using` environmental (thread local).
//! - Arguments are scale-encoded known types in the host.
//! - Host functions will decode the input and emit an event `with` environmental.

#![cfg_attr(not(feature = "std"), no_std)]
use sp_runtime_interface::runtime_interface;

#[cfg(feature = "std")]
use parity_scale_codec::Decode;
use sp_std::vec::Vec;

use evm_tracing_events::StepEventFilter;
#[cfg(feature = "std")]
use evm_tracing_events::{Event, EvmEvent, GasometerEvent, RuntimeEvent};

#[runtime_interface]
pub trait Ext {
	fn raw_step(&mut self, _data: Vec<u8>) {}

	fn raw_gas(&mut self, _data: Vec<u8>) {}

	fn raw_return_value(&mut self, _data: Vec<u8>) {}

	fn call_list_entry(&mut self, _index: u32, _value: Vec<u8>) {}

	fn call_list_new(&mut self) {}

	// New design, proxy events.
	/// An `Evm` event proxied by the Hippius runtime to this host function.
	/// evm -> runtime -> host.
	fn evm_event(&mut self, event: Vec<u8>) {
		if let Ok(event) = EvmEvent::decode(&mut &event[..]) {
			Event::Evm(event).emit();
		}
	}

	/// A `Gasometer` event proxied by the Hippius runtime to this host function.
	/// evm_gasometer -> runtime -> host.
	fn gasometer_event(&mut self, event: Vec<u8>) {
		if let Ok(event) = GasometerEvent::decode(&mut &event[..]) {
			Event::Gasometer(event).emit();
		}
	}

	/// A `Runtime` event proxied by the Hippius runtime to this host function.
	/// evm_runtime -> runtime -> host.
	fn runtime_event(&mut self, event: Vec<u8>) {
		if let Ok(event) = RuntimeEvent::decode(&mut &event[..]) {
			Event::Runtime(event).emit();
		}
	}

	/// Allow the tracing module in the runtime to know how to filter Step event
	/// content, as cloning the entire data is expensive and most of the time
	/// not necessary.
	fn step_event_filter() -> StepEventFilter {
		evm_tracing_events::step_event_filter().unwrap_or_default()
	}

	/// An event to create a new CallList (currently a new transaction when tracing a block).
	#[version(2)]
	fn call_list_new(&mut self) {
		Event::CallListNew().emit();
	}
}
