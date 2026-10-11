//! # Compile-affecting settings
//!
//! The settings that shape a compiled artifact are one value,
//! [`CompileOptions`], applied by one body, [`CompileOptions::configure`]:
//! the ahead-of-time compiler takes it explicitly, and the runtime's engine
//! applies the copy it loaded from the environment. That parity is what lets
//! the loading engine accept a pre-compiled artifact.

use wasmtime::Config;

/// The settings that shape a compiled artifact.
///
/// A pre-compiled component loads only into an engine configured with the
/// values it was compiled under, so these travel as one value. The default
/// matches the runtime's environment defaults, so an artifact compiled with
/// `CompileOptions::default()` loads into a runtime that sets none of the
/// compile-affecting variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    /// Per-invocation fuel budget; `0` disables metering (`MAX_FUEL`).
    pub max_fuel: u64,
    /// Honour WebAssembly branch hints (`BRANCH_HINTING`).
    pub branch_hinting: bool,
    /// Virtual address space reserved per linear memory, in bytes; `None`
    /// is Wasmtime's default (`MEMORY_RESERVATION`).
    pub memory_reservation: Option<u64>,
    /// Unmapped guard region after each linear memory, in bytes; `None` is
    /// Wasmtime's default (`MEMORY_GUARD_SIZE`).
    pub memory_guard_size: Option<u64>,
    /// Emit ELF symbol tables, for profilers and `wasmtime objdump`
    /// (`DEBUG_SYMBOLS`).
    pub debug_symbols: bool,
    /// Record the machine-code-to-wasm-offset map that gives traps and
    /// backtraces their wasm offsets (`GENERATE_ADDRESS_MAP`).
    pub generate_address_map: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            max_fuel: 0,
            branch_hinting: false,
            memory_reservation: None,
            memory_guard_size: None,
            debug_symbols: false,
            generate_address_map: true,
        }
    }
}

impl CompileOptions {
    /// Apply every compile-affecting setting to `config` — these six, and the
    /// two the runtime fixes (epoch interruption, copy-on-write heap images).
    ///
    /// The one body both the ahead-of-time compiler and the runtime's
    /// [`Config`] go through, so the two engines cannot disagree.
    pub fn configure(&self, config: &mut Config) {
        // epoch interruption: per-store deadlines drive cooperative guest timeouts
        config.epoch_interruption(true);

        // cow heap images are compile-affecting, so pinned explicitly
        config.memory_init_cow(true);

        if self.max_fuel > 0 {
            config.consume_fuel(true);
        }
        if self.branch_hinting {
            config.wasm_branch_hinting(true);
        }

        // memory tunables only when set, so unset keeps the wasmtime default
        if let Some(bytes) = self.memory_reservation {
            config.memory_reservation(bytes);
        }
        if let Some(bytes) = self.memory_guard_size {
            config.memory_guard_size(bytes);
        }

        // artifact size: strip symbols, keep the address map for trap offsets
        config.debug_symbols(self.debug_symbols);
        config.generate_address_map(self.generate_address_map);
    }
}
