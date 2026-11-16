//! Static checks of the executable artifact, `plugin.wasm`
//! (`docs/marketplace/artifact-contract.md`).
//!
//! A package of the `wasm` runtime ships one WebAssembly module that
//! implements plugin ABI 1 (`plugins/runtime/crates/abi`, module `guest`).
//! It is checked here, at verify time, without compiling or running it:
//!
//! * it is a valid core module under the WebAssembly 2.0 feature set (no
//!   components, threads, memory64, GC or exceptions);
//! * it imports exactly nothing but `deckpilot_v1.host_call` with type
//!   `(i32, i32) -> i64` (so no WASI and no other host function);
//! * it exports `memory` (its single, non-shared, 32-bit memory),
//!   `dp_abi_version: () -> i32`, `dp_alloc: (i32) -> i32` and
//!   `dp_invoke: (i32, i32) -> i64`; the optional `dp_describe: () -> i64`,
//!   `dp_init: () -> i32`, `dp_shutdown: () -> ()` and
//!   `dp_features: () -> i64` must have exactly these types when present;
//! * it has at most one table and is at most [`MAX_MODULE_BYTES`].
//!
//! A start function is allowed: the plugin host runs it under the same
//! deadline and fuel limits as an invocation, before `dp_abi_version`.
//! The host checks imports and exports again when it loads the module (it
//! trusts nothing the agent did not also check), and the agent checks the
//! installed bytes again before every load.

use std::fmt;

use wasmparser::types::EntityType;
use wasmparser::{FuncType, ValType, Validator, WasmFeatures};

/// Largest `plugin.wasm`. The host accepts up to 16 MiB, but a package is
/// at most 5 MiB compressed and a file at most 5 MiB unpacked.
pub const MAX_MODULE_BYTES: u64 = 4 * 1024 * 1024;

/// The only import module of ABI 1.
pub const IMPORT_MODULE: &str = "deckpilot_v1";
/// The only import of ABI 1.
pub const IMPORT_HOST_CALL: &str = "host_call";

/// Exports every module has: name and signature.
const REQUIRED_EXPORTS: [(&str, Signature); 3] = [
    ("dp_abi_version", Signature(&[], &[ValType::I32])),
    ("dp_alloc", Signature(&[ValType::I32], &[ValType::I32])),
    (
        "dp_invoke",
        Signature(&[ValType::I32, ValType::I32], &[ValType::I64]),
    ),
];

/// Exports a module may have (compatible additions within ABI 1).
const OPTIONAL_EXPORTS: [(&str, Signature); 4] = [
    ("dp_describe", Signature(&[], &[ValType::I64])),
    ("dp_init", Signature(&[], &[ValType::I32])),
    ("dp_shutdown", Signature(&[], &[])),
    ("dp_features", Signature(&[], &[ValType::I64])),
];

const HOST_CALL: Signature = Signature(&[ValType::I32, ValType::I32], &[ValType::I64]);

/// A function signature of ABI 1.
#[derive(Clone, Copy)]
struct Signature(&'static [ValType], &'static [ValType]);

impl Signature {
    fn matches(self, ty: &FuncType) -> bool {
        ty.params() == self.0 && ty.results() == self.1
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |types: &[ValType]| {
            types
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        write!(f, "({}) -> ({})", list(self.0), list(self.1))
    }
}

/// Why `plugin.wasm` does not implement plugin ABI 1. Messages name imports
/// and exports (at most 64 characters of each), never module contents.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModuleError {
    #[error("plugin.wasm is {size} bytes; the limit is {limit} bytes")]
    TooLarge { size: u64, limit: u64 },
    #[error("plugin.wasm is not a valid WebAssembly 2.0 core module: {0}")]
    Invalid(String),
    #[error(
        "plugin.wasm imports `{module}.{name}`; plugin ABI 1 modules may import only \
         `deckpilot_v1.host_call` (no WASI, no other host functions)"
    )]
    ForbiddenImport { module: String, name: String },
    #[error(
        "plugin.wasm imports `deckpilot_v1.host_call` with the wrong type (expected {expected})"
    )]
    HostCallType { expected: String },
    #[error("plugin.wasm does not export `{0}`, which plugin ABI 1 requires")]
    MissingExport(&'static str),
    #[error("plugin.wasm exports `{name}` with the wrong type (expected {expected})")]
    ExportType {
        name: &'static str,
        expected: String,
    },
    #[error(
        "plugin.wasm must have exactly one 32-bit, non-shared memory exported as `memory` ({0})"
    )]
    Memory(&'static str),
    #[error("plugin.wasm has {0} tables; plugin ABI 1 modules have at most one")]
    Tables(u32),
}

fn shown(text: &str) -> String {
    text.chars().take(64).collect()
}

/// The WebAssembly features a plugin module may use: WebAssembly 2.0 (sign
/// extension, saturating conversions, multi-value, reference types, bulk
/// memory, SIMD, mutable globals), which is what `wasm32-unknown-unknown`
/// produces by default and what the plugin host's wasmtime accepts.
pub fn features() -> WasmFeatures {
    WasmFeatures::WASM2
}

/// Checks that `bytes` is a plugin ABI 1 module (see the module docs).
pub fn validate_module(bytes: &[u8]) -> Result<(), ModuleError> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > MAX_MODULE_BYTES {
        return Err(ModuleError::TooLarge {
            size,
            limit: MAX_MODULE_BYTES,
        });
    }
    let mut validator = Validator::new_with_features(features());
    let types = validator
        .validate_all(bytes)
        .map_err(|e| ModuleError::Invalid(e.message().to_owned()))?;
    let types = types.as_ref();
    let func_type = |entity: EntityType| match entity {
        EntityType::Func(id) | EntityType::FuncExact(id) => match &types[id].composite_type.inner {
            wasmparser::CompositeInnerType::Func(ty) => Some(ty.clone()),
            _ => None,
        },
        _ => None,
    };

    let imports = types
        .core_imports()
        .ok_or_else(|| ModuleError::Invalid("a component, not a core module".into()))?;
    for (module, name, entity) in imports {
        if module != IMPORT_MODULE || name != IMPORT_HOST_CALL {
            return Err(ModuleError::ForbiddenImport {
                module: shown(module),
                name: shown(name),
            });
        }
        if !func_type(entity).is_some_and(|ty| HOST_CALL.matches(&ty)) {
            return Err(ModuleError::HostCallType {
                expected: HOST_CALL.to_string(),
            });
        }
    }

    if types.memory_count() != 1 {
        return Err(ModuleError::Memory("it has none or several"));
    }
    let memory = types.memory_at(0);
    if memory.memory64 || memory.shared {
        return Err(ModuleError::Memory("it is 64-bit or shared"));
    }
    if types.table_count() > 1 {
        return Err(ModuleError::Tables(types.table_count()));
    }

    let exports: Vec<(&str, EntityType)> = types
        .core_exports()
        .ok_or_else(|| ModuleError::Invalid("a component, not a core module".into()))?
        .collect();
    let export = |name: &str| {
        exports
            .iter()
            .find(|(export, _)| *export == name)
            .map(|(_, entity)| *entity)
    };
    match export("memory") {
        Some(EntityType::Memory(_)) => {}
        Some(_) => return Err(ModuleError::Memory("`memory` is not a memory")),
        None => return Err(ModuleError::MissingExport("memory")),
    }
    for (name, signature) in REQUIRED_EXPORTS {
        let entity = export(name).ok_or(ModuleError::MissingExport(name))?;
        if !func_type(entity).is_some_and(|ty| signature.matches(&ty)) {
            return Err(ModuleError::ExportType {
                name,
                expected: signature.to_string(),
            });
        }
    }
    for (name, signature) in OPTIONAL_EXPORTS {
        if let Some(entity) = export(name)
            && !func_type(entity).is_some_and(|ty| signature.matches(&ty))
        {
            return Err(ModuleError::ExportType {
                name,
                expected: signature.to_string(),
            });
        }
    }
    Ok(())
}

/// A minimal valid plugin ABI 1 module: it imports `host_call`, exports its
/// memory, `dp_abi_version` (returns 1), `dp_alloc` (returns a fixed
/// buffer) and `dp_invoke` (returns an empty result), and has no actions.
/// For tests and tools that need a structurally valid `plugin.wasm`; it
/// cannot run an action.
pub fn minimal_abi_module() -> Vec<u8> {
    encode::module(&encode::Spec::default())
}

/// [`minimal_abi_module`] with a custom section `note` holding `note`, so
/// that modules of different test plugins or versions differ.
pub fn minimal_abi_module_with_note(note: &str) -> Vec<u8> {
    encode::module(&encode::Spec {
        note: Some(note.to_owned()),
        ..encode::Spec::default()
    })
}

/// A tiny encoder for test modules (no dependency on a wasm toolchain).
#[doc(hidden)]
pub mod encode {
    /// What the generated module contains.
    #[derive(Debug, Clone, Default)]
    #[expect(
        clippy::struct_excessive_bools,
        reason = "independent switches of a test module generator"
    )]
    pub struct Spec {
        /// Extra imports `(module, name)` of type `() -> ()`.
        pub extra_imports: Vec<(String, String)>,
        /// Leave out the `host_call` import.
        pub no_host_call: bool,
        /// Exports to leave out (`memory`, `dp_abi_version`, `dp_alloc`,
        /// `dp_invoke`).
        pub omit: Vec<&'static str>,
        /// Export `dp_alloc` with type `() -> ()` instead.
        pub wrong_alloc_type: bool,
        /// A start function (calls nothing, returns).
        pub start: bool,
        /// Two memories (not valid without multi-memory).
        pub second_memory: bool,
        /// A custom section `note` with this text.
        pub note: Option<String>,
    }

    fn leb(mut value: u32, out: &mut Vec<u8>) {
        loop {
            let byte = u8::try_from(value & 0x7f).unwrap_or(0);
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn len(items: usize) -> u32 {
        u32::try_from(items).unwrap_or(u32::MAX)
    }

    fn name(text: &str, out: &mut Vec<u8>) {
        leb(len(text.len()), out);
        out.extend_from_slice(text.as_bytes());
    }

    fn section(id: u8, content: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        leb(len(content.len()), out);
        out.extend_from_slice(content);
    }

    /// Encodes `spec` as a binary module.
    pub fn module(spec: &Spec) -> Vec<u8> {
        const I32: u8 = 0x7f;
        const I64: u8 = 0x7e;
        // Types: 0 (i32 i32) -> i64, 1 () -> i32, 2 (i32) -> i32, 3 () -> ().
        let types: [&[u8]; 4] = [
            &[0x60, 2, I32, I32, 1, I64],
            &[0x60, 0, 1, I32],
            &[0x60, 1, I32, 1, I32],
            &[0x60, 0, 0],
        ];
        let mut out = b"\0asm\x01\0\0\0".to_vec();
        let mut content = Vec::new();
        leb(len(types.len()), &mut content);
        for ty in types {
            content.extend_from_slice(ty);
        }
        section(1, &content, &mut out);

        let mut imports: Vec<(&str, &str, u32)> = Vec::new();
        if !spec.no_host_call {
            imports.push(("deckpilot_v1", "host_call", 0));
        }
        for (module, field) in &spec.extra_imports {
            imports.push((module, field, 3));
        }
        let mut content = Vec::new();
        leb(len(imports.len()), &mut content);
        for (module, field, ty) in &imports {
            name(module, &mut content);
            name(field, &mut content);
            content.push(0);
            leb(*ty, &mut content);
        }
        section(2, &content, &mut out);

        // Functions: abi_version, alloc, invoke, start.
        let alloc_type = if spec.wrong_alloc_type { 3 } else { 2 };
        let functions = [1, alloc_type, 0, 3];
        let mut content = Vec::new();
        leb(len(functions.len()), &mut content);
        for ty in functions {
            leb(ty, &mut content);
        }
        section(3, &content, &mut out);

        let mut content = Vec::new();
        let memories = if spec.second_memory { 2 } else { 1 };
        leb(memories, &mut content);
        for _ in 0..memories {
            content.extend_from_slice(&[0, 1]);
        }
        section(5, &content, &mut out);

        let base = len(imports.len());
        let exports: Vec<(&str, u8, u32)> = [
            ("memory", 2, 0),
            ("dp_abi_version", 0, base),
            ("dp_alloc", 0, base + 1),
            ("dp_invoke", 0, base + 2),
        ]
        .into_iter()
        .filter(|(export, _, _)| !spec.omit.contains(export))
        .collect();
        let mut content = Vec::new();
        leb(len(exports.len()), &mut content);
        for (export, kind, index) in exports {
            name(export, &mut content);
            content.push(kind);
            leb(index, &mut content);
        }
        section(7, &content, &mut out);

        if spec.start {
            let mut content = Vec::new();
            leb(base + 3, &mut content);
            section(8, &content, &mut out);
        }

        let alloc_body: &[u8] = if spec.wrong_alloc_type {
            &[0, 0x0b]
        } else {
            &[0, 0x41, 0x80, 0x08, 0x0b]
        };
        let bodies: [&[u8]; 4] = [
            &[0, 0x41, 1, 0x0b],
            alloc_body,
            &[0, 0x42, 0, 0x0b],
            &[0, 0x0b],
        ];
        let mut content = Vec::new();
        leb(len(bodies.len()), &mut content);
        for body in bodies {
            leb(len(body.len()), &mut content);
            content.extend_from_slice(body);
        }
        section(10, &content, &mut out);
        if let Some(note) = &spec.note {
            let mut content = Vec::new();
            name("note", &mut content);
            content.extend_from_slice(note.as_bytes());
            section(0, &content, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::encode::{Spec, module};
    use super::*;

    #[test]
    fn the_minimal_module_is_valid() {
        validate_module(&minimal_abi_module()).unwrap();
        validate_module(&module(&Spec {
            start: true,
            ..Spec::default()
        }))
        .unwrap();
        // A module that never calls the host needs no import.
        validate_module(&module(&Spec {
            no_host_call: true,
            ..Spec::default()
        }))
        .unwrap();
    }

    #[test]
    fn forbidden_imports_are_named() {
        for (m, n) in [
            ("wasi_snapshot_preview1", "fd_write"),
            ("env", "memcpy"),
            ("deckpilot_v1", "get_database_key"),
            ("deckpilot_v2", "host_call"),
        ] {
            let bytes = module(&Spec {
                extra_imports: vec![(m.into(), n.into())],
                ..Spec::default()
            });
            assert_eq!(
                validate_module(&bytes),
                Err(ModuleError::ForbiddenImport {
                    module: m.into(),
                    name: n.into()
                })
            );
        }
        let duplicate = module(&Spec {
            extra_imports: vec![("deckpilot_v1".into(), "host_call".into())],
            ..Spec::default()
        });
        assert!(matches!(
            validate_module(&duplicate),
            Err(ModuleError::HostCallType { .. })
        ));
    }

    #[test]
    fn exports_are_required_with_their_types() {
        for name in ["dp_abi_version", "dp_alloc", "dp_invoke"] {
            let bytes = module(&Spec {
                omit: vec![name],
                ..Spec::default()
            });
            assert_eq!(
                validate_module(&bytes),
                Err(ModuleError::MissingExport(name))
            );
        }
        let bytes = module(&Spec {
            omit: vec!["memory"],
            ..Spec::default()
        });
        assert_eq!(
            validate_module(&bytes),
            Err(ModuleError::MissingExport("memory"))
        );
        let bytes = module(&Spec {
            wrong_alloc_type: true,
            ..Spec::default()
        });
        assert_eq!(
            validate_module(&bytes),
            Err(ModuleError::ExportType {
                name: "dp_alloc",
                expected: "(i32) -> (i32)".into()
            })
        );
    }

    #[test]
    fn malformed_and_oversized_modules_are_refused() {
        assert!(matches!(
            validate_module(b"console.log('hi')"),
            Err(ModuleError::Invalid(_))
        ));
        let mut truncated = minimal_abi_module();
        truncated.truncate(truncated.len() - 3);
        assert!(matches!(
            validate_module(&truncated),
            Err(ModuleError::Invalid(_))
        ));
        assert!(matches!(
            validate_module(&module(&Spec {
                second_memory: true,
                ..Spec::default()
            })),
            Err(ModuleError::Invalid(_) | ModuleError::Memory(_))
        ));
        let big = vec![0u8; usize::try_from(MAX_MODULE_BYTES).unwrap() + 1];
        assert!(matches!(
            validate_module(&big),
            Err(ModuleError::TooLarge { .. })
        ));
    }
}
