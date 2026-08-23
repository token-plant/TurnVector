mod audit_journal;
mod backend_contract;
mod certification_tooling;
mod control_plane;
mod control_store;
mod daemon_custody;
mod data_plane;
mod device_executor;
mod event_loop;
mod fake_backend;
mod native_build;
mod native_runtime;
mod native_turns;
mod protocol_authority;
mod residency_coordinator;
mod resource_evidence;
mod resource_governor;
mod runtime_carry;
mod runtime_measurement;
mod runtime_qualification;
mod volume_qualification;

#[cfg(test)]
mod core_gate;
#[cfg(test)]
mod fault_gate;
#[cfg(test)]
mod lifecycle_gate;
#[cfg(test)]
mod qualification_core_adapters;
#[cfg(test)]
mod qualification_integration;
#[cfg(test)]
mod qualification_lifecycle_adapters;
#[cfg(test)]
mod qualification_system_adapters;
#[cfg(test)]
mod scheduling_gate;

fn main() {}
