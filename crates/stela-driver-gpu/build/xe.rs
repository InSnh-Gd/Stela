//! xe: the `xe-y` composite of its Kbuild, compiled unchanged in Stela's
//! Linux environment.
//!
//! # Where the port stands
//!
//! Every object of `xe-y` compiles, archives, and links against an environment
//! that supplies every service it reaches except the traps [`OPEN`] lists:
//! [`check_imports`] refuses to hand over an archive whose imports anything
//! would have to answer at the first device access, so the build passing means
//! the composite's reachable kernel services are accounted for. What the
//! environment gained for it:
//!
//! - **Managed devices**: `drivers/base/devres.c` whole, and with it the
//!   device core's `devm_ioremap_wc`, `device_create_file` and
//!   `devm_device_add_group`. Stela's device model no longer keeps its own
//!   action list: `stela_linux_devres_release` is upstream's
//!   `devres_release_all`.
//! - **PCI**: `pcim_enable_device`, `pci_iomap`/`pci_iounmap`,
//!   `pci_bus_resource_n`, `pci_pme_capable`, the resizable-BAR size helpers
//!   and `pci_resize_resource`, plus MSI over the owner's own
//!   `enable_msi` service: `pci_alloc_irq_vectors` returns the one message the
//!   owner moves, and `pci_msix_vec_count` reports no MSI-X table, which is
//!   what makes xe take its MSI path.
//! - **Memory and DMA**: `dma_map_resource`/`dma_unmap_resource`.
//! - **Library and clock**: `lib/cmdline.c`, `drivers/gpu/buddy.c` (the
//!   local-memory allocator `xe_ttm_vram_mgr` carves the card up with),
//!   `simple_strtoull`, `ktime_get_raw` and `down_read_interruptible`.
//! - **Runtime PM**: with `CONFIG_PM` off, Linux's stand-ins answer `-EINVAL`
//!   and 0 to `pm_runtime_get_if_active()` and `pm_runtime_get_if_in_use()`,
//!   and a driver reads that as "this device is asleep" and drops the work an
//!   interrupt asked it to do. xe's G2H receive and its CT fast path both ask
//!   (`xe_guc_ct.c`), so against the stand-ins the GuC handshake times out
//!   while every interrupt is delivered (the CT shows the GuC consuming the
//!   H2G, with no G2H written back). A device Stela claims is powered, and
//!   stays powered while its owner runs, so those two questions now answer
//!   what a PM-enabled kernel says for a device that never suspends
//!   (`include/linux/pm_runtime.h`, under `CONFIG_STELA`); nothing counts the
//!   reference either way, since every put in this configuration is upstream's
//!   stub.
//! - **Traps**: the groups in [`OPEN`]. They are unreachable in Stela rather
//!   than merely unimplemented: the GSC firmware interface only exists on the
//!   discrete parts, the I2C-attached AMC only in a display build, and the
//!   rest need an address space or an interrupt-in-service state Stela's
//!   entries never create.
//!
//! The wiring is in place: `engine/gpu.rs` claims the function for this port
//! (`claim_intel`) and runs the image, the loader hands xe the GuC and HuC
//! images out of the host firmware bundle, and the owner supplies the
//! function's BARs and its single message interrupt. xe then opens the command
//! transport on both of the part's graphics tiles, and the boot's `GPU START
//! claim_ms=… linux_ms=… probe_ms=…` line reports it started: the sandbox's
//! serial log carries `Initialized xe 1.1.0 for 0000:00:02.0 on minor 0` and
//! the GPU step completes instead of failing.
//!
//! # The composite
//!
//! [`Kbuild::read`] reads `xe-y` and its `i915-display/%.o` rule; the
//! out-of-band workaround tables are rendered here rather than by the generator
//! its Makefile runs, and [`generate`] holds the templates that output follows.
use super::kbuild::{Config, Include, Kbuild};
use super::native::Linux;
use object::{Object, ObjectSymbol, read::archive::ArchiveFile};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    thread,
};
use stela_c_build::linux::Module;
use stela_c_build::linux_capture::{self as initcalls, Projection, Unit, UnitSpec};

const XE: &str = "../../third_party/linux/drivers/gpu/drm/xe";
const TREE: &str = "../../third_party/linux";
/// The display code xe shares with i915, which its Kbuild compiles from the
/// i915 tree through a `$(obj)/i915-display/%.o` rule.
const I915_DISPLAY: &str = "drivers/gpu/drm/i915/display";
const MAKEFILE: &str = "Makefile";
const AUTOCONF: &str = "../stela-linux/src/compat/generated/autoconf.h";
const KERNEL: &str = "../stela-linux/src/compat/kernel";
const MODULE: Module<'static> = Module {
    name: "xe",
    file: "drivers/gpu/drm/xe/xe",
};
/// Kbuild's composite: xe's DRM device over its graphics and media engines.
const COMPOSITES: [&str; 1] = ["xe"];
/// The Kbuild lines no object list depends on: the module object itself (named
/// once in [`MODULE`]), the KUnit test tree, warning flags, the generator host
/// program and the rules that render the workaround tables Stela renders
/// itself, and the header tests.
const UNREFERENCED: [&str; 13] = [
    "obj-",
    "subdir-ccflags-",
    "hostprogs",
    "generated_oob",
    "generated_device_oob",
    "quiet_cmd_",
    "cmd_",
    "$(obj)/",
    "$(call ",
    "hdrtest_find_args",
    "always-",
    "uses_generated_oob",
    "$(uses_generated_oob)",
];

/// The rule files `xe_gen_wa_oob` reads and the generated pair it writes for
/// each. The generator derives the enumerator prefix from the header's name.
const OOB: [&str; 2] = ["xe_wa_oob", "xe_device_wa_oob"];

/// What the compiler itself calls, which every platform's runtime supplies.
const RUNTIME: [&str; 4] = ["memcpy", "memmove", "memset", "memcmp"];
/// The prefix of what Stela's Rust side exports to the environment.
const PLATFORM: &str = "stela_linux_";
/// The prefix of the hosted test harness (`stela/test.h`). Only a hosted
/// environment built with its source fixtures reaches it, from fixture code
/// sharing archive members with the services xe imports.
const HARNESS: &str = "stela_test_";

/// The kernel services the composite imports that Stela's Linux environment
/// lacks, or defines only as a trap, by service. Building fails when the
/// imports change, so each supplied service leaves this list with its change.
///
/// Only the traps are left: every other service xe reaches is implemented, so
/// this list is the set of `STELA_UNSUPPORTED` declarations in `stela-linux`
/// that the composite's objects reach.
const OPEN: &[(&str, &[&str])] = &[
    (
        "GSC firmware interface (only DG1/DG2/PVC build one)",
        &[
            "auxiliary_device_init",
            "__auxiliary_device_add",
            "__irq_alloc_descs",
            "irq_free_descs",
            "irq_set_chip_and_handler_name",
            "irq_set_chip_data",
            "handle_simple_irq",
            "generic_handle_irq_safe",
        ],
    ),
    (
        "I2C-attached AMC of the display engine (a display build only)",
        &[
            "i2c_verify_adapter",
            "i2c_transfer_buffer_flags",
            "bus_register_notifier",
            "bus_unregister_notifier",
            "fwnode_create_software_node",
            "fwnode_remove_software_node",
            "platform_device_alloc",
            "platform_device_add",
            "platform_device_put",
            "platform_device_unregister",
        ],
    ),
    (
        "per-client entry points Stela serves nothing through",
        &[
            "mmput",
            "kthread_use_mm",
            "kthread_unuse_mm",
            "vmf_insert_pfn",
            "anon_inode_getfd",
            "get_cmdline",
            "pci_assign_unassigned_bus_resources",
        ],
    ),
    (
        "shmem pages and a freed path (carried over from the nouveau port)",
        &[
            "shmem_read_folio_gfp",
            "shmem_read_mapping_page_gfp",
            "shmem_truncate_range",
            "shmem_writeout",
            "folio_clear_dirty_for_io",
            "mm_get_unmapped_area",
            "stela_linux_reopen_unowned_path",
        ],
    ),
];

/// The unit of xe's own `module_init`.
const INITCALLS: UnitSpec = UnitSpec {
    scope: stela_c_build::linux_capture::Scope {
        slice: "gpu",
        group: "xe",
        group_ordinal: 9,
        unit_ordinal: 1,
        global_ordinal: 9,
    },
    source: "drivers/gpu/drm/xe/xe_module",
    module: MODULE.name,
    file: MODULE.file,
};
fn initcalls_unit() -> Unit {
    INITCALLS.into()
}

pub fn port(linux: &mut Linux) {
    let config = Config::parse(&fs::read_to_string(AUTOCONF).expect("read Stela's Kconfig header"))
        .unwrap_or_else(|error| panic!("{AUTOCONF}: {error}"));
    let kbuild = Kbuild::read(Path::new(XE), MAKEFILE, &config, &COMPOSITES)
        .unwrap_or_else(|error| panic!("{error}"));
    for file in &kbuild.files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let unreviewed = kbuild
        .ignored
        .iter()
        .filter(|line| !UNREFERENCED.iter().any(|prefix| line.starts_with(prefix)))
        .collect::<Vec<_>>();
    assert!(
        unreviewed.is_empty(),
        "xe's Makefile gained {unreviewed:#?}"
    );
    let objects = kbuild
        .objects("xe")
        .unwrap_or_else(|error| panic!("{error}"));
    let sources = objects
        .iter()
        .map(|object| source(object))
        .collect::<Vec<_>>();
    let generated = generate(&stela_c_build::linux_build::out_dir().join("xe"));
    let includes = includes(&kbuild, &generated);
    let mut sources = sources;
    // Compiled below, with its registrations captured.
    sources.retain(|source| *source != initcalls_unit().path());
    let mut build = stela_c_build::native::configure();
    linux
        .environment
        .apply(&mut build, Some(&generated), &includes, Some(&MODULE));
    stela_c_build::linux_build::upstream_warnings(&mut build);
    // As Kbuild builds every profile: Linux relies on the optimizer to drop
    // the calls its configuration disables.
    build.opt_level(2).cargo_metadata(false);
    let jobs = env::var("NUM_JOBS")
        .ok()
        .and_then(|jobs| jobs.parse().ok())
        .unwrap_or(1)
        .max(1);
    let objects = thread::scope(|scope| {
        sources
            .chunks(sources.len().div_ceil(jobs))
            .map(|chunk| {
                let build = &build;
                scope.spawn(move || build.clone().files(chunk).compile_intermediates())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|compiling| compiling.join().expect("a compiling thread"))
            .collect::<Vec<_>>()
    });
    let mut captured = linux.cohort();
    let environment_count = captured.len();
    let compiler = initcalls::compiler(&build);
    let placement = initcalls::Placement::new(
        &initcalls_unit().path(),
        &stela_c_build::linux_build::out_dir(),
        &[
            Path::new("../.."),
            compiler.parent().expect("compiler root"),
        ],
    )
    .expect("xe source placement");
    captured.push(initcalls::compile(
        &build,
        Path::new("../stela-linux/src/compat/stela/registration.h"),
        INITCALLS,
        &placement,
    ));
    let mut expected = super::native::environment_units();
    expected.push(INITCALLS);
    let image = initcalls::source(
        &captured,
        Projection {
            symbol: "stela_linux_xe_image",
            expected: &expected,
            imported_units: environment_count,
            descriptors_only: false,
        },
    );
    build
        .objects(objects)
        .objects(
            captured
                .iter()
                .skip(environment_count)
                .map(|unit| unit.object()),
        )
        .file(image)
        .compile("stela-xe");
    let archive = stela_c_build::linux_build::out_dir().join("libstela-xe.a");
    check_imports(&archive, &[&linux.archive, &linux.common_archive]);
    let output = stela_c_build::linux_build::out_dir();
    let digest = initcalls::write(
        &output.join("captured-units.bin"),
        &captured[initcalls::COMMON.len()..],
    );
    println!("cargo:output={}", output.display());
    println!("cargo:cohort_sha256={digest}");
    // Hosted programs test the environment and link no driver.
    if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "uefi") {
        println!("cargo:rustc-link-lib=static=stela-xe");
    }
}

/// The source of one `xe-y` object, which the display rule reaches into the
/// i915 tree for everything under `i915-display/`.
fn source(object: &str) -> PathBuf {
    match object.strip_prefix("i915-display/") {
        Some(below) => Path::new(TREE)
            .join(I915_DISPLAY)
            .join(below)
            .with_extension("c"),
        None => Path::new(XE).join(object).with_extension("c"),
    }
}

/// xe's include directories, with the generated workaround tables where
/// `-I$(obj)` puts them.
fn includes(kbuild: &Kbuild, generated: &Path) -> Vec<PathBuf> {
    kbuild
        .includes
        .iter()
        .map(|include| match include {
            Include::Source(below) => Path::new(XE).join(below),
            Include::Tree(below) => Path::new(TREE).join(below),
            Include::Object => generated.to_owned(),
        })
        .collect()
}

/// Write the generated out-of-band workaround tables `xe_gen_wa_oob` renders
/// from the rule files, and return the directory holding them.
///
/// The host program runs no part of the build: its output is plain text from
/// the rule files, so Stela renders the same bytes where Kbuild would.
fn generate(out: &Path) -> PathBuf {
    let generated = out.join("generated");
    fs::create_dir_all(&generated).expect("create xe's generated directory");
    for name in OOB {
        let rules = fs::read_to_string(Path::new(XE).join(format!("{name}.rules")))
            .unwrap_or_else(|error| panic!("{name}.rules: {error}"));
        let entries = rules_of(&rules);
        // The generator names its enumerators after the header's file name.
        let prefix = format!("{name}.h")
            .split('.')
            .next()
            .expect("a header name")
            .to_uppercase();
        for (file, text) in [
            (
                format!("{name}.h"),
                enum_of(&prefix, &format!("{name}.rules"), &entries),
            ),
            (format!("{name}.c"), table_of(&entries)),
        ] {
            fs::write(generated.join(&file), text)
                .unwrap_or_else(|error| panic!("{file}: {error}"));
        }
    }
    out.to_owned()
}

/// The named rules of a `*_wa_oob.rules` file: one entry per line that starts
/// a name, continued by the lines that are indented under it.
fn rules_of(rules: &str) -> Vec<(String, Vec<String>)> {
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();
    for (number, line) in rules.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let continuation = line.starts_with(char::is_whitespace);
        let text = line.trim_start();
        if continuation {
            let entry = entries
                .last_mut()
                .unwrap_or_else(|| panic!("{}:{}: continuation without a name", rules, number + 1));
            assert!(!text.is_empty(), "{}:{}: empty rule", rules, number + 1);
            entry.1.push(text.to_owned());
            continue;
        }
        let (name, rule) = text
            .split_once(char::is_whitespace)
            .unwrap_or_else(|| panic!("{}:{}: no rule", rules, number + 1));
        let rule = rule.trim_start();
        assert!(!rule.is_empty(), "{}:{}: empty rule", rules, number + 1);
        entries.push((name.to_owned(), vec![rule.to_owned()]));
    }
    entries
}

/// `generated/<name>.h`, as the host program's HEADER and FOOTER templates
/// render it.
fn enum_of(prefix: &str, source: &str, entries: &[(String, Vec<String>)]) -> String {
    let mut text = format!(
        "// SPDX-License-Identifier: MIT\n\
         \n\
         /*\n\
         \x20* DO NOT MODIFY.\n\
         \x20*\n\
         \x20* This file was generated from rules: {source}\n\
         \x20*/\n\
         #ifndef _GENERATED_{prefix}_\n\
         #define _GENERATED_{prefix}_\n\
         \n\
         enum {{\n"
    );
    for (index, (name, _)) in entries.iter().enumerate() {
        text.push_str(&format!("\t{prefix}_{name} = {index},\n"));
    }
    text.push_str(&format!("\t_{prefix}_COUNT = {}\n", entries.len()));
    text.push_str("};\n\n#endif\n");
    text
}

/// `generated/<name>.c`, as the host program renders it.
fn table_of(entries: &[(String, Vec<String>)]) -> String {
    let mut text = String::new();
    for (name, rules) in entries {
        text.push_str(&format!(
            "{{ XE_RTP_NAME(\"{name}\"),\n  XE_RTP_RULES({}",
            rules[0]
        ));
        for rule in &rules[1..] {
            text.push_str(&format!(", OR,\n\t{rule}"));
        }
        text.push_str(") },\n");
    }
    text
}

/// Compare the services the composite reaches when linked, directly or
/// through the environment's objects, and that nothing supplies, with
/// [`OPEN`].
fn check_imports(port: &Path, environment: &[&Path]) {
    let port = members(port);
    let environment = environment
        .iter()
        .flat_map(|archive| members(archive))
        .collect::<Vec<_>>();
    let definers = environment
        .iter()
        .flat_map(|member| member.defined.iter().map(move |symbol| (symbol, member)))
        .collect::<BTreeMap<_, _>>();
    let defined = port
        .iter()
        .flat_map(|member| &member.defined)
        .collect::<BTreeSet<_>>();
    let traps = traps();
    let harness = env::var_os("DEP_STELA_LINUX_HOSTED_FIXTURES").is_some();
    let mut reached = port
        .iter()
        .flat_map(|member| &member.imported)
        .filter(|symbol| !defined.contains(symbol))
        .collect::<Vec<_>>();
    let mut seen = reached.iter().copied().collect::<BTreeSet<_>>();
    let mut open = BTreeSet::new();
    while let Some(symbol) = reached.pop() {
        if traps.contains(symbol) {
            open.insert(symbol.clone());
        }
        match definers.get(symbol) {
            Some(member) => reached.extend(
                member
                    .imported
                    .iter()
                    .filter(|&import| !defined.contains(import) && seen.insert(import)),
            ),
            None if RUNTIME.contains(&symbol.as_str())
                || symbol.starts_with(PLATFORM)
                || (harness && symbol.starts_with(HARNESS)) => {}
            None => {
                open.insert(symbol.clone());
            }
        }
    }
    let mut stale = Vec::new();
    for symbol in OPEN.iter().flat_map(|(_, symbols)| symbols.iter()) {
        if !open.remove(*symbol) {
            stale.push(*symbol);
        }
    }
    assert!(
        open.is_empty() && stale.is_empty(),
        "xe's imports changed: newly open {open:?}; now supplied {stale:?}"
    );
}

/// What one archive member defines and imports.
struct Member {
    defined: BTreeSet<String>,
    imported: BTreeSet<String>,
}

fn members(archive: &Path) -> Vec<Member> {
    let data = fs::read(archive).unwrap_or_else(|error| panic!("{}: {error}", archive.display()));
    let archive = ArchiveFile::parse(&*data).expect("an archive");
    archive
        .members()
        .map(|member| {
            let member = member.expect("an archive member");
            let object = object::File::parse(member.data(&*data).expect("member data"))
                .expect("an object file");
            let (mut defined, mut imported) = (BTreeSet::new(), BTreeSet::new());
            for symbol in object.symbols() {
                let Ok(name) = symbol.name() else {
                    continue;
                };
                if symbol.is_undefined() {
                    imported.insert(name.to_owned());
                } else if symbol.is_global() {
                    defined.insert(name.to_owned());
                }
            }
            Member { defined, imported }
        })
        .collect()
}

/// What the environment defines without serving: functions that trap
/// (`STELA_UNSUPPORTED`), placeholder objects (`STELA_UNSUPPORTED_OBJECT`)
/// and functions that report themselves unsupported and fail.
fn traps() -> BTreeSet<String> {
    let mut traps = BTreeSet::new();
    for source in stela_c_build::linux_build::sources(KERNEL)
        .into_iter()
        .chain(stela_c_build::linux_build::sources(
            "src/compat/linux/kernel",
        ))
    {
        let text = fs::read_to_string(&source).expect("read a kernel-API source");
        for (marker, field) in [
            ("STELA_UNSUPPORTED(", 1),
            ("STELA_UNSUPPORTED_OBJECT(", 1),
            ("stela_linux_unsupported(\"", 0),
        ] {
            for definition in text.split(marker).skip(1) {
                let end = definition.find([')', '"']).unwrap_or(definition.len());
                if let Some(name) = definition[..end].split(',').nth(field) {
                    traps.insert(name.trim().to_owned());
                }
            }
        }
    }
    traps
}
