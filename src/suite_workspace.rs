//! Staged suite workspace builder for opted-in `dev_deps` collection.
//!
//! `session` calls this builder for suites whose `build_targets` opt into
//! collector prebuilds. Suites that omit `build_targets` stay on the default
//! dylib build path.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Suite;
use crate::dylib::{self, BuildOutput};
use crate::error::{Error, Outcome};

const COLLECTOR_PACKAGE: &str = "__lihaaf_dev_deps_collector";

/// Parameters needed to synthesize and build an opted-in suite workspace.
#[derive(Debug, Clone)]
pub struct BuildParams<'a> {
    /// `dylib_crate` from `[package.metadata.lihaaf]`.
    pub dylib_crate: &'a str,
    /// The final resolved suite whose `dev_deps` and `features` are built.
    pub suite: &'a crate::config::Suite,
    /// Path to the metadata crate's `Cargo.toml`.
    pub metadata_manifest_path: &'a std::path::Path,
    /// The suite-specific lihaaf target directory.
    pub target_dir: &'a std::path::Path,
    /// Captured rustc identity for parity with the default build API.
    pub toolchain: &'a crate::toolchain::Toolchain,
}

/// Build the staged dylib package and synthetic dev-deps collector.
pub fn build(params: &BuildParams<'_>) -> Result<BuildOutput, Error> {
    validate_entry_guard(params.suite)?;

    let plan = synthesize_workspace(
        params.dylib_crate,
        params.suite,
        params.metadata_manifest_path,
        params.target_dir,
    )?;

    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg(params.dylib_crate)
        .arg("-p")
        .arg(COLLECTOR_PACKAGE)
        .arg("--release")
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&plan.root_manifest)
        .arg("--target-dir")
        .arg(params.target_dir);

    for feature in &params.suite.features {
        cmd.arg("--features")
            .arg(format!("{}/{}", params.dylib_crate, feature));
    }

    let prior_rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let rustflags = if prior_rustflags.is_empty() {
        "-C prefer-dynamic".to_string()
    } else {
        format!("{prior_rustflags} -C prefer-dynamic")
    };
    cmd.env("RUSTFLAGS", &rustflags);

    let feature_args = params
        .suite
        .features
        .iter()
        .map(|f| format!(" --features {}/{}", params.dylib_crate, f))
        .collect::<String>();
    let invocation = format!(
        "RUSTFLAGS={:?} cargo build -p {} -p {} --release \
         --message-format=json-render-diagnostics --manifest-path {:?} --target-dir {:?}{}",
        rustflags,
        params.dylib_crate,
        COLLECTOR_PACKAGE,
        plan.root_manifest,
        params.target_dir,
        feature_args
    );

    let output = cmd.output().map_err(|e| Error::SubprocessSpawn {
        program: "cargo".to_string(),
        source: e,
    })?;

    if !output.status.success() {
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        append_offline_lockfile_note(&mut stderr, &plan);
        return Err(Error::Session(Outcome::DylibBuildFailed {
            invocation,
            stderr,
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let cargo_dylib_path =
        dylib::parse_dylib_path(&stdout, params.dylib_crate).ok_or_else(|| {
            Error::Session(Outcome::DylibNotFound {
                invocation: invocation.clone(),
                crate_name: params.dylib_crate.to_string(),
            })
        })?;

    let _ = params.toolchain;

    Ok(BuildOutput {
        cargo_dylib_path,
        deps_dir: params.target_dir.join("release/deps"),
        invocation,
    })
}

#[derive(Debug)]
struct Synthesis {
    root_manifest: PathBuf,
    collector_manifest: PathBuf,
    staged_dylib_manifest: PathBuf,
    staged_path_dep_manifests: Vec<PathBuf>,
    copied_lockfile: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct ManifestDoc {
    path: PathBuf,
    dir: PathBuf,
    table: toml::value::Table,
}

#[derive(Debug, Clone)]
struct WorkspaceContext {
    manifest: PathBuf,
    dir: PathBuf,
    table: toml::value::Table,
}

#[derive(Debug, Clone)]
struct StagedMember {
    package_name: String,
    source_manifest: PathBuf,
    member_dir: PathBuf,
    manifest_path: PathBuf,
}

struct SynthesisContext {
    dylib_crate: String,
    workspace_dir: PathBuf,
    workspace_dependencies: toml::value::Table,
    staged_by_source: BTreeMap<PathBuf, StagedMember>,
    staged_names: BTreeMap<String, PathBuf>,
    staged_path_dep_manifests: Vec<PathBuf>,
}

fn synthesize_workspace(
    dylib_crate: &str,
    suite: &Suite,
    metadata_manifest_path: &Path,
    target_dir: &Path,
) -> Result<Synthesis, Error> {
    validate_entry_guard(suite)?;

    let metadata = read_manifest(metadata_manifest_path)?;
    let metadata_package = package_name(&metadata)?;
    let workspace_context = find_workspace_context(&metadata.dir)?;
    let dylib_manifest = resolve_dylib_manifest(dylib_crate, &metadata, &workspace_context)?;
    let workspace_dir = target_dir.join("lihaaf-suite-workspace");

    crate::util::remove_path_race_free(&workspace_dir, "suite workspace")?;
    std::fs::create_dir_all(&workspace_dir)
        .map_err(|e| Error::io(e, "creating suite workspace", Some(workspace_dir.clone())))?;

    let copied_lockfile = copy_ancestor_lockfile(&metadata.dir, &workspace_dir)?;

    let mut root_workspace = build_root_workspace_table(&workspace_context, dylib_crate)?;
    let workspace_dependencies = root_workspace
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();

    let mut ctx = SynthesisContext {
        dylib_crate: dylib_crate.to_string(),
        workspace_dir: workspace_dir.clone(),
        workspace_dependencies,
        staged_by_source: BTreeMap::new(),
        staged_names: BTreeMap::new(),
        staged_path_dep_manifests: Vec::new(),
    };

    if workspace_context
        .as_ref()
        .is_none_or(|workspace| workspace.manifest != dylib_manifest.path)
    {
        reject_member_local_overrides(&dylib_manifest.table, &dylib_manifest.dir)?;
    }
    let mut dylib_table = stage_manifest_table(&dylib_manifest, StageKind::Dylib)?;
    if metadata_package != dylib_crate {
        reject_package_workspace_split_roots(&metadata, &workspace_context)?;
    }
    ctx.rewrite_dependency_sections(
        &mut dylib_table,
        &dylib_manifest.dir,
        &workspace_dir.join("dylib"),
        CurrentPackage::Dylib,
    )?;

    let collector_dependencies = collector_dependencies(suite, &metadata, &mut ctx)?;
    let collector_table = collector_manifest_table(collector_dependencies);

    if !ctx.workspace_dependencies.is_empty() {
        root_workspace.insert(
            "dependencies".to_string(),
            toml::Value::Table(ctx.workspace_dependencies.clone()),
        );
    }

    let mut root = toml::value::Table::new();
    root_workspace.insert(
        "members".to_string(),
        toml::Value::Array(workspace_members(&ctx)),
    );
    root.insert("workspace".to_string(), toml::Value::Table(root_workspace));
    carry_workspace_root_top_level(&mut root, &workspace_context, dylib_crate)?;

    let root_manifest = workspace_dir.join("Cargo.toml");
    let staged_dylib_manifest = workspace_dir.join("dylib/Cargo.toml");
    let collector_manifest = workspace_dir.join("collector/Cargo.toml");
    write_manifest(&root_manifest, &root)?;
    write_manifest(&staged_dylib_manifest, &dylib_table)?;
    write_manifest(&collector_manifest, &collector_table)?;
    write_file(&workspace_dir.join("collector/src/lib.rs"), b"")?;

    Ok(Synthesis {
        root_manifest,
        collector_manifest,
        staged_dylib_manifest,
        staged_path_dep_manifests: ctx.staged_path_dep_manifests,
        copied_lockfile,
    })
}

fn validate_entry_guard(suite: &Suite) -> Result<(), Error> {
    let is_tests_target = suite
        .build_targets
        .as_slice()
        .iter()
        .map(String::as_str)
        .eq(["tests"]);
    if !is_tests_target {
        return Err(config_invalid(format!(
            "suite \"{}\" reached staged suite workspace build without \
             build_targets = [\"tests\"]. This path is only valid for opted-in suites.",
            suite.name
        )));
    }
    if suite.dev_deps.is_empty() {
        return Err(config_invalid(format!(
            "suite \"{}\" sets build_targets = [\"tests\"] but has no final dev_deps. \
             There is no fixture dependency graph to collect.",
            suite.name
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageKind {
    Dylib,
    PathDep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurrentPackage {
    Dylib,
    Other,
}

impl SynthesisContext {
    fn rewrite_dependency_sections(
        &mut self,
        manifest: &mut toml::value::Table,
        source_dir: &Path,
        staged_dir: &Path,
        current: CurrentPackage,
    ) -> Result<(), Error> {
        reject_member_local_overrides(manifest, source_dir)?;
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(deps) = manifest
                .get_mut(section)
                .and_then(toml::Value::as_table_mut)
            {
                self.rewrite_dependency_table(deps, source_dir, staged_dir, current)?;
            }
        }

        if let Some(targets) = manifest
            .get_mut("target")
            .and_then(toml::Value::as_table_mut)
        {
            for (_, target) in targets.iter_mut() {
                let Some(target_table) = target.as_table_mut() else {
                    continue;
                };
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(deps) = target_table
                        .get_mut(section)
                        .and_then(toml::Value::as_table_mut)
                    {
                        self.rewrite_dependency_table(deps, source_dir, staged_dir, current)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn rewrite_dependency_table(
        &mut self,
        deps: &mut toml::value::Table,
        source_dir: &Path,
        staged_dir: &Path,
        current: CurrentPackage,
    ) -> Result<(), Error> {
        let keys = deps.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let Some(value) = deps.get_mut(&key) else {
                continue;
            };
            self.rewrite_dependency_value(&key, value, source_dir, staged_dir, current)?;
        }
        Ok(())
    }

    fn rewrite_dependency_value(
        &mut self,
        key: &str,
        value: &mut toml::Value,
        source_dir: &Path,
        staged_dir: &Path,
        current: CurrentPackage,
    ) -> Result<(), Error> {
        match value {
            toml::Value::String(_) => {
                if key == self.dylib_crate {
                    return Err(config_invalid(format!(
                        "registry dependency `{key}` names dylib_crate `{}` in staged graph; \
                         registry/git self-edges are not supported for staged suite workspaces.",
                        self.dylib_crate
                    )));
                }
            }
            toml::Value::Table(spec) => {
                if spec.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                    self.select_workspace_dependency(key)?;
                    return Ok(());
                }

                let package = spec.get("package").and_then(toml::Value::as_str);
                let has_git = spec.contains_key("git");
                let has_path = spec.contains_key("path");
                if !has_path {
                    if key == self.dylib_crate || package == Some(self.dylib_crate.as_str()) {
                        let source = if has_git { "git" } else { "registry" };
                        return Err(config_invalid(format!(
                            "{source} dependency `{key}` names dylib_crate `{}` in staged graph; \
                             registry/git self-edges are not supported.",
                            self.dylib_crate
                        )));
                    }
                    return Ok(());
                }

                let path = spec
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| {
                        config_invalid(format!(
                            "path dependency `{key}` has non-string `path`; lihaaf cannot \
                             analyze this back-edge safely."
                        ))
                    })?;
                let dep_dir = absolutize_path(source_dir, Path::new(path));
                let dep_manifest = dep_dir.join("Cargo.toml");
                let dep = read_manifest(&dep_manifest).map_err(|e| {
                    config_invalid(format!(
                        "path dependency `{key}` at `{}` cannot be analyzed safely: {e}",
                        dep_manifest.display()
                    ))
                })?;
                let dep_name = package_name(&dep).map_err(|e| {
                    config_invalid(format!(
                        "path dependency `{key}` at `{}` cannot be analyzed safely: {e}",
                        dep_manifest.display()
                    ))
                })?;

                if dep_name == self.dylib_crate {
                    if current == CurrentPackage::Dylib {
                        return Err(config_invalid(format!(
                            "staged dylib manifest has a path dependency `{key}` back to \
                             `{}`; lihaaf cannot rewrite a self-cycle safely.",
                            self.dylib_crate
                        )));
                    }
                    spec.insert(
                        "path".to_string(),
                        toml::Value::String(relative_path_string(staged_dir, &self.dylib_dir())),
                    );
                    return Ok(());
                }

                let staged = self.stage_path_dependency(dep)?;
                spec.insert(
                    "path".to_string(),
                    toml::Value::String(relative_path_string(staged_dir, &staged.member_dir)),
                );
            }
            _ => {
                return Err(config_invalid(format!(
                    "dependency `{key}` uses an unsupported TOML shape; staged suite workspace \
                     synthesis supports string and table dependency specs."
                )));
            }
        }
        Ok(())
    }

    fn select_workspace_dependency(&mut self, key: &str) -> Result<(), Error> {
        let mut value = self
            .workspace_dependencies
            .get(key)
            .cloned()
            .ok_or_else(|| {
                config_invalid(format!(
                    "dependency `{key}` uses `workspace = true`, but the staged workspace has no \
                 `[workspace.dependencies.{key}]` entry to carry."
                ))
            })?;
        let workspace_dir = self.workspace_dir.clone();
        self.rewrite_dependency_value(
            key,
            &mut value,
            &workspace_dir,
            &workspace_dir,
            CurrentPackage::Other,
        )?;
        self.workspace_dependencies.insert(key.to_string(), value);
        Ok(())
    }

    fn stage_path_dependency(&mut self, dep: ManifestDoc) -> Result<StagedMember, Error> {
        let canonical = canonical_manifest_key(&dep.path)?;
        if let Some(existing) = self.staged_by_source.get(&canonical) {
            return Ok(existing.clone());
        }

        let package_name = package_name(&dep)?;
        if let Some(other_source) = self.staged_names.get(&package_name)
            && other_source != &canonical
        {
            return Err(config_invalid(format!(
                "multiple staged path dependencies use package name `{package_name}`; lihaaf \
                 cannot safely synthesize an unambiguous workspace package graph."
            )));
        }

        let member_dir = self
            .workspace_dir
            .join("staged-path-deps")
            .join(sanitize_member_dir(&package_name));
        let manifest_path = member_dir.join("Cargo.toml");
        let staged = StagedMember {
            package_name: package_name.clone(),
            source_manifest: canonical.clone(),
            member_dir: member_dir.clone(),
            manifest_path: manifest_path.clone(),
        };
        self.staged_by_source
            .insert(canonical.clone(), staged.clone());
        self.staged_names.insert(package_name, canonical);

        reject_member_local_overrides(&dep.table, &dep.dir)?;
        let mut table = stage_manifest_table(&dep, StageKind::PathDep)?;
        self.rewrite_dependency_sections(&mut table, &dep.dir, &member_dir, CurrentPackage::Other)?;
        write_manifest(&manifest_path, &table)?;
        self.staged_path_dep_manifests.push(manifest_path);

        Ok(staged)
    }

    fn dylib_dir(&self) -> PathBuf {
        self.workspace_dir.join("dylib")
    }
}

fn collector_dependencies(
    suite: &Suite,
    metadata: &ManifestDoc,
    ctx: &mut SynthesisContext,
) -> Result<toml::value::Table, Error> {
    let dev_deps = metadata
        .table
        .get("dev-dependencies")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();
    let mut out = toml::value::Table::new();

    for dep in &suite.dev_deps {
        let Some(mut spec) = dev_deps.get(dep).cloned() else {
            if target_specific_dev_dep_exists(&metadata.table, dep) {
                return Err(config_invalid(format!(
                    "dev_dep `{dep}` exists only in target-specific dev-dependencies; \
                     staged suite workspaces support only top-level [dev-dependencies]."
                )));
            }
            return Err(config_invalid(format!(
                "dev_dep `{dep}` is missing from top-level [dev-dependencies] in `{}`.",
                metadata.path.display()
            )));
        };

        reject_renamed_or_optional_dev_dep(dep, &spec)?;
        ctx.rewrite_dependency_value(
            dep,
            &mut spec,
            &metadata.dir,
            &ctx.workspace_dir.join("collector"),
            CurrentPackage::Other,
        )?;
        out.insert(dep.clone(), spec);
    }

    Ok(out)
}

fn collector_manifest_table(dependencies: toml::value::Table) -> toml::value::Table {
    let mut package = toml::value::Table::new();
    package.insert(
        "name".to_string(),
        toml::Value::String(COLLECTOR_PACKAGE.to_string()),
    );
    package.insert(
        "version".to_string(),
        toml::Value::String("0.0.0".to_string()),
    );
    package.insert(
        "edition".to_string(),
        toml::Value::String("2021".to_string()),
    );
    package.insert("publish".to_string(), toml::Value::Boolean(false));

    let mut lib = toml::value::Table::new();
    lib.insert(
        "path".to_string(),
        toml::Value::String("src/lib.rs".to_string()),
    );

    let mut root = toml::value::Table::new();
    root.insert("package".to_string(), toml::Value::Table(package));
    root.insert("lib".to_string(), toml::Value::Table(lib));
    root.insert("dependencies".to_string(), toml::Value::Table(dependencies));
    root
}

fn stage_manifest_table(
    manifest: &ManifestDoc,
    kind: StageKind,
) -> Result<toml::value::Table, Error> {
    reject_build_script(manifest)?;

    let mut table = manifest.table.clone();
    table.remove("bin");
    table.remove("example");
    table.remove("test");
    table.remove("bench");
    table.remove("dev-dependencies");
    table.remove("workspace");
    table.remove("patch");
    table.remove("replace");
    table
        .entry("dependencies".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));

    if let Some(package) = table.get_mut("package").and_then(toml::Value::as_table_mut) {
        package.remove("workspace");
        for key in ["autobins", "autoexamples", "autotests", "autobenches"] {
            package.insert(key.to_string(), toml::Value::Boolean(false));
        }
    }

    let lib_path = table
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("src/lib.rs"));
    let absolute_lib_path = absolutize_path(&manifest.dir, &lib_path);

    let lib = table
        .entry("lib".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            config_invalid(format!(
                "`[lib]` in `{}` is not a table; lihaaf cannot stage the package.",
                manifest.path.display()
            ))
        })?;
    lib.insert(
        "path".to_string(),
        toml::Value::String(absolute_lib_path.to_string_lossy().into_owned()),
    );
    if kind == StageKind::Dylib {
        ensure_dylib_crate_types(lib);
    }

    Ok(table)
}

fn ensure_dylib_crate_types(lib: &mut toml::value::Table) {
    let mut seen = BTreeSet::new();
    let mut values = Vec::new();
    for required in ["rlib", "dylib"] {
        seen.insert(required.to_string());
        values.push(toml::Value::String(required.to_string()));
    }
    if let Some(existing) = lib.get("crate-type").and_then(toml::Value::as_array) {
        for value in existing {
            let Some(kind) = value.as_str() else {
                continue;
            };
            if seen.insert(kind.to_string()) {
                values.push(toml::Value::String(kind.to_string()));
            }
        }
    }
    lib.insert("crate-type".to_string(), toml::Value::Array(values));
}

fn build_root_workspace_table(
    ctx: &Option<WorkspaceContext>,
    _dylib_crate: &str,
) -> Result<toml::value::Table, Error> {
    let mut workspace = toml::value::Table::new();
    if let Some(ctx) = ctx {
        let source_ws = ctx
            .table
            .get("workspace")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| {
                config_invalid(format!(
                    "`{}` was identified as a workspace root but has no [workspace] table.",
                    ctx.manifest.display()
                ))
            })?;
        for key in ["resolver", "package", "lints", "metadata", "dependencies"] {
            if let Some(value) = source_ws.get(key).cloned() {
                workspace.insert(key.to_string(), value);
            }
        }
        if let Some(package) = workspace
            .get_mut("package")
            .and_then(toml::Value::as_table_mut)
        {
            absolutize_workspace_package_paths(package, &ctx.dir)?;
        }
        if let Some(deps) = workspace
            .get_mut("dependencies")
            .and_then(toml::Value::as_table_mut)
        {
            absolutize_dependency_table_paths(deps, &ctx.dir)?;
        }
    }
    Ok(workspace)
}

fn carry_workspace_root_top_level(
    staged_root: &mut toml::value::Table,
    ctx: &Option<WorkspaceContext>,
    dylib_crate: &str,
) -> Result<(), Error> {
    let Some(ctx) = ctx else {
        return Ok(());
    };
    if let Some(mut replace) = ctx.table.get("replace").cloned() {
        absolutize_replace_paths(&mut replace, &ctx.dir)?;
        staged_root.insert("replace".to_string(), replace);
    }
    if let Some(value) = ctx.table.get("profile").cloned() {
        staged_root.insert("profile".to_string(), value);
    }

    if let Some(patch) = ctx.table.get("patch").cloned() {
        let mut patch = patch.as_table().cloned().ok_or_else(|| {
            config_invalid(format!(
                "`[patch]` in `{}` is not a table; staged suite workspace cannot carry it.",
                ctx.manifest.display()
            ))
        })?;
        for (_, registry) in patch.iter_mut() {
            let entries = registry.as_table_mut().ok_or_else(|| {
                config_invalid(format!(
                    "`[patch]` registry in `{}` is not a table.",
                    ctx.manifest.display()
                ))
            })?;
            let keys = entries.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let Some(entry) = entries.get_mut(&key) else {
                    continue;
                };
                if key == dylib_crate
                    || entry
                        .as_table()
                        .and_then(|t| t.get("package"))
                        .and_then(toml::Value::as_str)
                        == Some(dylib_crate)
                {
                    return Err(config_invalid(format!(
                        "workspace-root patch entry `{key}` names dylib_crate `{dylib_crate}`; \
                         staged suite workspaces reject self patches."
                    )));
                }
                if let Some(table) = entry.as_table_mut()
                    && let Some(path) = table.get("path").and_then(toml::Value::as_str)
                {
                    table.insert(
                        "path".to_string(),
                        toml::Value::String(
                            absolutize_path(&ctx.dir, Path::new(path))
                                .to_string_lossy()
                                .into_owned(),
                        ),
                    );
                }
            }
        }
        staged_root.insert("patch".to_string(), toml::Value::Table(patch));
    }
    Ok(())
}

fn workspace_members(ctx: &SynthesisContext) -> Vec<toml::Value> {
    let mut members = vec![
        toml::Value::String("dylib".to_string()),
        toml::Value::String("collector".to_string()),
    ];
    for staged in ctx.staged_by_source.values() {
        members.push(toml::Value::String(format!(
            "staged-path-deps/{}",
            staged
                .member_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&staged.package_name)
        )));
    }
    members
}

fn reject_renamed_or_optional_dev_dep(dep: &str, spec: &toml::Value) -> Result<(), Error> {
    let Some(table) = spec.as_table() else {
        return Ok(());
    };
    if table.contains_key("package") {
        return Err(config_invalid(format!(
            "dev_dep `{dep}` uses `package = ...`; renamed collector dependencies need extern \
             alias support and are not implemented in v0.1.0."
        )));
    }
    if table.get("optional").and_then(toml::Value::as_bool) == Some(true) {
        return Err(config_invalid(format!(
            "dev_dep `{dep}` is optional; collector feature-forcing semantics are not \
             implemented in v0.1.0."
        )));
    }
    Ok(())
}

fn reject_build_script(manifest: &ManifestDoc) -> Result<(), Error> {
    let package = manifest
        .table
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| {
            config_invalid(format!(
                "`{}` has no [package] table; lihaaf cannot stage it.",
                manifest.path.display()
            ))
        })?;
    match package.get("build") {
        Some(toml::Value::Boolean(false)) => Ok(()),
        Some(_) => Err(config_invalid(format!(
            "staged package `{}` declares a build script in `{}`; build-script staging is not \
             implemented for suite workspaces.",
            package_name(manifest)?,
            manifest.path.display()
        ))),
        None if manifest.dir.join("build.rs").exists() => Err(config_invalid(format!(
            "staged package `{}` has default `build.rs` at `{}`; build-script staging is not \
             implemented for suite workspaces.",
            package_name(manifest)?,
            manifest.dir.join("build.rs").display()
        ))),
        None => Ok(()),
    }
}

fn reject_member_local_overrides(
    manifest: &toml::value::Table,
    source_dir: &Path,
) -> Result<(), Error> {
    if manifest.contains_key("patch") {
        return Err(config_invalid(format!(
            "member-local [patch.*] in `{}` is not supported by staged suite workspaces.",
            source_dir.join("Cargo.toml").display()
        )));
    }
    if manifest.contains_key("replace") {
        return Err(config_invalid(format!(
            "member-local [replace] in `{}` is not supported by staged suite workspaces.",
            source_dir.join("Cargo.toml").display()
        )));
    }
    Ok(())
}

fn reject_self_edges_in_dependency_table(
    deps: &toml::value::Table,
    dylib_crate: &str,
) -> Result<(), Error> {
    for (key, value) in deps {
        let package = value
            .as_table()
            .and_then(|t| t.get("package"))
            .and_then(toml::Value::as_str);
        let is_path = value
            .as_table()
            .and_then(|t| t.get("path"))
            .and_then(toml::Value::as_str)
            .is_some();
        if !is_path && (key == dylib_crate || package == Some(dylib_crate)) {
            return Err(config_invalid(format!(
                "workspace dependency `{key}` names dylib_crate `{dylib_crate}` through a \
                 registry/git source; staged suite workspaces reject registry/git self-edges."
            )));
        }
    }
    Ok(())
}

fn absolutize_workspace_package_paths(
    package: &mut toml::value::Table,
    base: &Path,
) -> Result<(), Error> {
    for key in ["readme", "license-file"] {
        if let Some(value) = package.get_mut(key) {
            match value {
                toml::Value::String(path) => {
                    *path = absolutize_path(base, Path::new(path))
                        .to_string_lossy()
                        .into_owned();
                }
                toml::Value::Boolean(_) if key == "readme" => {}
                _ => {
                    return Err(config_invalid(format!(
                        "workspace.package.{key} must be a string path{} for staged suite \
                         workspace carry-down.",
                        if key == "readme" { " or boolean" } else { "" }
                    )));
                }
            }
        }
    }
    Ok(())
}

fn absolutize_replace_paths(value: &mut toml::Value, base: &Path) -> Result<(), Error> {
    let replace = value.as_table_mut().ok_or_else(|| {
        config_invalid("[replace] must be a table for staged suite workspace carry-down.")
    })?;
    for (key, entry) in replace.iter_mut() {
        let table = entry.as_table_mut().ok_or_else(|| {
            config_invalid(format!(
                "[replace] entry `{key}` must be a table for staged suite workspace carry-down."
            ))
        })?;
        if let Some(path) = table.get("path").and_then(toml::Value::as_str) {
            table.insert(
                "path".to_string(),
                toml::Value::String(
                    absolutize_path(base, Path::new(path))
                        .to_string_lossy()
                        .into_owned(),
                ),
            );
        } else if table.contains_key("path") {
            return Err(config_invalid(format!(
                "[replace] entry `{key}` has non-string `path`; lihaaf cannot carry it safely."
            )));
        }
    }
    Ok(())
}

fn absolutize_dependency_table_paths(
    deps: &mut toml::value::Table,
    base: &Path,
) -> Result<(), Error> {
    for (key, value) in deps {
        let Some(table) = value.as_table_mut() else {
            continue;
        };
        if let Some(path) = table.get("path").and_then(toml::Value::as_str) {
            table.insert(
                "path".to_string(),
                toml::Value::String(
                    absolutize_path(base, Path::new(path))
                        .to_string_lossy()
                        .into_owned(),
                ),
            );
        } else if table.contains_key("path") {
            return Err(config_invalid(format!(
                "dependency `{key}` has non-string `path`; lihaaf cannot analyze it safely."
            )));
        }
    }
    Ok(())
}

fn target_specific_dev_dep_exists(root: &toml::value::Table, dep: &str) -> bool {
    root.get("target")
        .and_then(toml::Value::as_table)
        .map(|targets| {
            targets.values().any(|target| {
                target
                    .get("dev-dependencies")
                    .and_then(toml::Value::as_table)
                    .is_some_and(|deps| deps.contains_key(dep))
            })
        })
        .unwrap_or(false)
}

fn resolve_dylib_manifest(
    dylib_crate: &str,
    metadata: &ManifestDoc,
    workspace_context: &Option<WorkspaceContext>,
) -> Result<ManifestDoc, Error> {
    if package_name(metadata)? == dylib_crate {
        return Ok(metadata.clone());
    }
    let Some(workspace) = workspace_context else {
        return Err(config_invalid(format!(
            "dylib_crate `{dylib_crate}` differs from metadata package `{}` but no ancestor \
             virtual workspace was found to resolve the split-crate shape.",
            package_name(metadata)?
        )));
    };
    if workspace.table.contains_key("package") {
        return Err(config_invalid(format!(
            "dylib_crate `{dylib_crate}` differs from the metadata package, but ancestor \
             workspace `{}` also has [package]; package+workspace split-crate roots are not \
             supported by the staged suite workspace resolver.",
            workspace.manifest.display()
        )));
    }

    let (manifest, _) =
        crate::compat::overlay::resolve_workspace_member_manifest(&workspace.manifest, dylib_crate)
            .map_err(|e| {
                config_invalid(format!(
                    "could not resolve dylib_crate `{dylib_crate}` from ancestor virtual \
                     workspace `{}`: {e}",
                    workspace.manifest.display()
                ))
            })?;
    read_manifest(&manifest)
}

fn reject_package_workspace_split_roots(
    metadata: &ManifestDoc,
    workspace_context: &Option<WorkspaceContext>,
) -> Result<(), Error> {
    if let Some(workspace) = workspace_context
        && workspace.manifest == metadata.path
        && metadata.table.contains_key("workspace")
    {
        return Err(config_invalid(format!(
            "split-crate staged suite workspace resolution does not support package+workspace \
             roots like `{}`.",
            metadata.path.display()
        )));
    }
    Ok(())
}

fn find_workspace_context(start_dir: &Path) -> Result<Option<WorkspaceContext>, Error> {
    for dir in start_dir.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let doc = read_manifest(&manifest)?;
        if doc.table.contains_key("workspace") {
            return Ok(Some(WorkspaceContext {
                manifest,
                dir: dir.to_path_buf(),
                table: doc.table,
            }));
        }
    }
    Ok(None)
}

fn copy_ancestor_lockfile(
    start_dir: &Path,
    workspace_dir: &Path,
) -> Result<Option<PathBuf>, Error> {
    for dir in start_dir.ancestors() {
        let lockfile = dir.join("Cargo.lock");
        if lockfile.exists() {
            std::fs::copy(&lockfile, workspace_dir.join("Cargo.lock")).map_err(|e| {
                Error::io(
                    e,
                    "copying ancestor Cargo.lock into suite workspace",
                    Some(lockfile.clone()),
                )
            })?;
            return Ok(Some(lockfile));
        }
    }
    Ok(None)
}

fn read_manifest(path: &Path) -> Result<ManifestDoc, Error> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::io(e, "reading Cargo.toml", Some(path.to_path_buf())))?;
    let value: toml::Value =
        toml::from_str(&text).map_err(|e: toml::de::Error| Error::TomlParse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
    let table = value.as_table().cloned().ok_or_else(|| {
        config_invalid(format!(
            "`{}` did not parse to a TOML table.",
            path.display()
        ))
    })?;
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(ManifestDoc {
        path: path.to_path_buf(),
        dir,
        table,
    })
}

fn write_manifest(path: &Path, table: &toml::value::Table) -> Result<(), Error> {
    let bytes = toml::to_string_pretty(table)
        .map_err(|e| Error::JsonParse {
            context: format!("serializing staged manifest `{}`", path.display()),
            message: e.to_string(),
        })?
        .into_bytes();
    write_file(path, &bytes)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    crate::util::write_file_atomic(path, bytes)
}

fn append_offline_lockfile_note(stderr: &mut String, plan: &Synthesis) {
    if !looks_like_ambient_offline_failure(stderr) {
        return;
    }
    let staged_lockfile = plan
        .root_manifest
        .parent()
        .map(|dir| dir.join("Cargo.lock"));
    stderr.push_str("\n\nlihaaf: staged suite workspace note: Cargo appears to have failed under ambient offline/cache policy. ");
    if let (Some(source), Some(staged)) = (&plan.copied_lockfile, staged_lockfile) {
        stderr.push_str(&format!(
            "lihaaf copied the ancestor Cargo.lock from `{}` to `{}`; updates stay inside the temporary suite workspace. ",
            source.display(),
            staged.display()
        ));
    } else {
        stderr.push_str("No ancestor Cargo.lock was copied into the temporary suite workspace. ");
    }
    stderr.push_str(
        "Warm Cargo's registry/git cache for the collector dependencies, or remove \
         build_targets = [\"tests\"] for this suite in environments without registry access.",
    );
}

fn looks_like_ambient_offline_failure(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("offline")
        || stderr.contains("no matching package named")
        || stderr.contains("failed to download")
        || stderr.contains("could not download")
        || stderr.contains("failed to fetch")
}

fn package_name(manifest: &ManifestDoc) -> Result<String, Error> {
    manifest
        .table
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            config_invalid(format!(
                "`{}` has no [package].name; lihaaf cannot stage it.",
                manifest.path.display()
            ))
        })
}

fn canonical_manifest_key(path: &Path) -> Result<PathBuf, Error> {
    std::fs::canonicalize(path).map_err(|e| {
        Error::io(
            e,
            "canonicalizing staged path dependency manifest",
            Some(path.to_path_buf()),
        )
    })
}

fn absolutize_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn relative_path_string(from_dir: &Path, to_dir: &Path) -> String {
    let Ok(from_rel) = from_dir.strip_prefix(common_workspace_root(from_dir, to_dir)) else {
        return to_dir.to_string_lossy().into_owned();
    };
    let Ok(to_rel) = to_dir.strip_prefix(common_workspace_root(from_dir, to_dir)) else {
        return to_dir.to_string_lossy().into_owned();
    };
    let mut parts = Vec::new();
    for _ in from_rel.components() {
        parts.push("..".to_string());
    }
    for component in to_rel.components() {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn common_workspace_root<'a>(a: &'a Path, b: &'a Path) -> &'a Path {
    let mut last = Path::new("");
    for ancestor in a.ancestors() {
        if b.starts_with(ancestor) {
            last = ancestor;
            break;
        }
    }
    last
}

fn sanitize_member_dir(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn config_invalid(message: impl Into<String>) -> Error {
    Error::Session(Outcome::ConfigInvalid {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::synthesize_workspace;
    use crate::config::{DEFAULT_COMPILE_FAIL_MARKER, DEFAULT_EDITION, DEFAULT_SUITE_NAME, Suite};
    use crate::error::{Error, Outcome};
    use crate::normalize::Substitution;

    fn suite(dev_deps: &[&str]) -> Suite {
        Suite {
            name: DEFAULT_SUITE_NAME.to_string(),
            extern_crates: vec!["core".to_string()],
            fixture_dirs: vec![PathBuf::from("tests/lihaaf")],
            features: Vec::new(),
            edition: DEFAULT_EDITION.to_string(),
            dev_deps: dev_deps.iter().map(|s| (*s).to_string()).collect(),
            build_targets: vec!["tests".to_string()]
                .try_into()
                .expect("valid build target"),
            compile_fail_marker: DEFAULT_COMPILE_FAIL_MARKER.to_string(),
            fixture_timeout_secs: 90,
            per_fixture_memory_mb: 1024,
            allow_lints: Vec::new(),
            extra_substitutions: Vec::<Substitution>::new(),
            strip_lines: Vec::new(),
            strip_line_prefixes: Vec::new(),
            keep_foreign_span_bodies: false,
        }
    }

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("test path has parent"))
            .expect("create test parent");
        std::fs::write(path, contents).expect("write test file");
    }

    fn read_toml(path: &Path) -> toml::Value {
        toml::from_str(&std::fs::read_to_string(path).expect("read toml")).expect("parse toml")
    }

    fn table<'a>(value: &'a toml::Value, key: &str) -> &'a toml::value::Table {
        value
            .get(key)
            .and_then(toml::Value::as_table)
            .unwrap_or_else(|| panic!("missing table `{key}` in {value:#?}"))
    }

    fn dep_table<'a>(
        value: &'a toml::Value,
        table_name: &str,
        dep: &str,
    ) -> &'a toml::value::Table {
        table(value, table_name)
            .get(dep)
            .and_then(toml::Value::as_table)
            .unwrap_or_else(|| panic!("missing dependency `{dep}` in table `{table_name}`"))
    }

    fn config_invalid_message(err: Error) -> String {
        match err {
            Error::Session(Outcome::ConfigInvalid { message }) => message,
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn entry_guard_rejects_non_opted_or_empty_dev_deps_before_manifest_read() {
        let tmp = TempDir::new().expect("tempdir");

        let mut not_opted = suite(&["helper"]);
        not_opted.build_targets = Default::default();
        let err = synthesize_workspace(
            "core",
            &not_opted,
            &tmp.path().join("missing/Cargo.toml"),
            tmp.path(),
        )
        .unwrap_err();
        assert!(config_invalid_message(err).contains("build_targets"));

        let err = synthesize_workspace(
            "core",
            &suite(&[]),
            &tmp.path().join("missing/Cargo.toml"),
            tmp.path(),
        )
        .unwrap_err();
        assert!(config_invalid_message(err).contains("dev_deps"));
    }

    #[test]
    fn collector_deps_are_metadata_dev_deps_and_paths_are_absolutized() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        let metadata_manifest = root.join("metadata/Cargo.toml");
        let local_dep = root.join("metadata/local-helper");
        write(
            &metadata_manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dev-dependencies]
helper = { path = "local-helper", features = ["derive"], default-features = false }
serde = "1"
"#,
        );
        write(&root.join("metadata/src/lib.rs"), "");
        write(
            &local_dep.join("Cargo.toml"),
            r#"
[package]
name = "helper"
version = "0.1.0"
edition = "2021"
"#,
        );
        write(&local_dep.join("src/lib.rs"), "");

        let out = synthesize_workspace(
            "core",
            &suite(&["helper", "serde"]),
            &metadata_manifest,
            root,
        )
        .expect("synthesis succeeds");

        let collector = read_toml(&out.collector_manifest);
        let helper = dep_table(&collector, "dependencies", "helper");
        assert_eq!(
            helper.get("path").and_then(toml::Value::as_str),
            Some("../staged-path-deps/helper")
        );
        assert_eq!(
            helper
                .get("default-features")
                .and_then(toml::Value::as_bool),
            Some(false)
        );
        assert!(
            table(&collector, "dependencies").contains_key("serde"),
            "registry dev-dep should be copied to collector dependencies"
        );
        let staged_helper = out
            .staged_path_dep_manifests
            .iter()
            .find(|p| {
                table(&read_toml(p), "package")
                    .get("name")
                    .and_then(toml::Value::as_str)
                    == Some("helper")
            })
            .expect("helper staged");
        assert_eq!(
            table(&read_toml(staged_helper), "lib")
                .get("path")
                .and_then(toml::Value::as_str),
            Some(
                local_dep
                    .join("src/lib.rs")
                    .to_str()
                    .expect("utf8 temp path")
            )
        );

        let dylib = read_toml(&out.staged_dylib_manifest);
        assert!(
            !table(&dylib, "dependencies").contains_key("helper"),
            "metadata dev-deps must not be promoted into staged dylib dependencies"
        );
    }

    #[test]
    fn invalid_dev_dep_shapes_reject_before_cargo() {
        let tmp = TempDir::new().expect("tempdir");
        let manifest = tmp.path().join("Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
renamed = { package = "actual", version = "1" }
optional_dep = { version = "1", optional = true }

[target.'cfg(unix)'.dev-dependencies]
target_only = "1"
"#,
        );
        write(&tmp.path().join("src/lib.rs"), "");

        let missing =
            synthesize_workspace("core", &suite(&["missing"]), &manifest, tmp.path()).unwrap_err();
        assert!(config_invalid_message(missing).contains("missing"));

        let target_only =
            synthesize_workspace("core", &suite(&["target_only"]), &manifest, tmp.path())
                .unwrap_err();
        assert!(config_invalid_message(target_only).contains("target-specific"));

        let renamed =
            synthesize_workspace("core", &suite(&["renamed"]), &manifest, tmp.path()).unwrap_err();
        let renamed_msg = config_invalid_message(renamed);
        assert!(renamed_msg.contains("renamed"));
        assert!(renamed_msg.contains("package"));

        let optional =
            synthesize_workspace("core", &suite(&["optional_dep"]), &manifest, tmp.path())
                .unwrap_err();
        let optional_msg = config_invalid_message(optional);
        assert!(optional_msg.contains("optional_dep"));
        assert!(optional_msg.contains("optional"));
    }

    #[test]
    fn staged_dylib_has_absolute_lib_path_crate_types_and_strips_targets() {
        let tmp = TempDir::new().expect("tempdir");
        let manifest = tmp.path().join("Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"
autobins = true
autoexamples = true
autotests = true
autobenches = true

[lib]
path = "src/custom.rs"
name = "core_lib"
crate-type = ["cdylib"]

[[bin]]
name = "cli"
path = "src/main.rs"

[[example]]
name = "demo"
path = "examples/demo.rs"

[[test]]
name = "smoke"
path = "tests/smoke.rs"

[[bench]]
name = "bench"
path = "benches/bench.rs"

[dev-dependencies]
helper = "1"
"#,
        );
        write(&tmp.path().join("src/custom.rs"), "");

        let out = synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path())
            .expect("synthesis succeeds");
        let staged = read_toml(&out.staged_dylib_manifest);
        let lib = table(&staged, "lib");

        assert_eq!(
            lib.get("path").and_then(toml::Value::as_str),
            Some(
                tmp.path()
                    .join("src/custom.rs")
                    .to_str()
                    .expect("utf8 temp path")
            )
        );
        let crate_types: Vec<_> = lib
            .get("crate-type")
            .and_then(toml::Value::as_array)
            .expect("crate-type array")
            .iter()
            .map(|v| v.as_str().expect("string crate-type"))
            .collect();
        assert!(crate_types.contains(&"rlib"));
        assert!(crate_types.contains(&"dylib"));
        assert!(crate_types.contains(&"cdylib"));
        assert_eq!(
            table(&staged, "package")
                .get("autobins")
                .and_then(toml::Value::as_bool),
            Some(false)
        );
        assert!(staged.get("bin").is_none());
        assert!(staged.get("example").is_none());
        assert!(staged.get("test").is_none());
        assert!(staged.get("bench").is_none());
    }

    #[test]
    fn staged_dylib_strips_source_workspace_table() {
        let tmp = TempDir::new().expect("tempdir");
        let manifest = tmp.path().join("Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[workspace]
members = ["."]
resolver = "2"

[dev-dependencies]
helper = "1"
"#,
        );
        write(&tmp.path().join("src/lib.rs"), "");

        let out = synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path())
            .expect("synthesis succeeds");
        let staged = read_toml(&out.staged_dylib_manifest);

        assert!(
            staged.get("workspace").is_none(),
            "staged workspace members must not retain their source `[workspace]` table"
        );
    }

    #[test]
    fn build_scripts_reject_for_staged_packages() {
        let tmp = TempDir::new().expect("tempdir");
        let manifest = tmp.path().join("Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"
build = "build.rs"

[dev-dependencies]
helper = "1"
"#,
        );
        write(&tmp.path().join("src/lib.rs"), "");
        let err =
            synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path()).unwrap_err();
        assert!(config_invalid_message(err).contains("build script"));

        let implicit = tmp.path().join("implicit/Cargo.toml");
        write(
            &implicit,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
helper = "1"
"#,
        );
        write(&tmp.path().join("implicit/src/lib.rs"), "");
        write(&tmp.path().join("implicit/build.rs"), "");
        let err =
            synthesize_workspace("core", &suite(&["helper"]), &implicit, tmp.path()).unwrap_err();
        assert!(config_invalid_message(err).contains("build.rs"));
    }

    #[test]
    fn path_back_edge_to_dylib_crate_is_rewritten_to_staged_member() {
        let tmp = TempDir::new().expect("tempdir");
        let manifest = tmp.path().join("core/Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
helper = { path = "../helper" }
"#,
        );
        write(&tmp.path().join("core/src/lib.rs"), "");
        write(
            &tmp.path().join("helper/Cargo.toml"),
            r#"
[package]
name = "helper"
version = "0.1.0"
edition = "2021"

[dependencies]
core = { path = "../core", features = ["macros"] }
"#,
        );
        write(&tmp.path().join("helper/src/lib.rs"), "");

        let out = synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path())
            .expect("synthesis succeeds");
        let staged_helper = out
            .staged_path_dep_manifests
            .iter()
            .find(|p| {
                table(&read_toml(p), "package")
                    .get("name")
                    .and_then(toml::Value::as_str)
                    == Some("helper")
            })
            .expect("helper staged");
        let helper = read_toml(staged_helper);
        let core = dep_table(&helper, "dependencies", "core");
        assert_eq!(
            core.get("path").and_then(toml::Value::as_str),
            Some("../../dylib")
        );
        assert_eq!(
            core.get("features")
                .and_then(toml::Value::as_array)
                .expect("features")
                .len(),
            1
        );
    }

    #[test]
    fn registry_or_git_self_edges_reject() {
        let tmp = TempDir::new().expect("tempdir");
        let manifest = tmp.path().join("core/Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
helper = { path = "../helper" }
"#,
        );
        write(&tmp.path().join("core/src/lib.rs"), "");
        write(
            &tmp.path().join("helper/Cargo.toml"),
            r#"
[package]
name = "helper"
version = "0.1.0"
edition = "2021"

[dependencies]
core = "1"
"#,
        );
        write(&tmp.path().join("helper/src/lib.rs"), "");

        let err =
            synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path()).unwrap_err();
        let msg = config_invalid_message(err);
        assert!(msg.contains("registry") || msg.contains("git"));
        assert!(msg.contains("core"));
    }

    #[test]
    fn workspace_root_patches_are_carried_and_self_patches_reject() {
        let tmp = TempDir::new().expect("tempdir");
        let ws_manifest = tmp.path().join("Cargo.toml");
        write(
            &ws_manifest,
            r#"
[workspace]
members = ["core"]
resolver = "2"

[patch.crates-io]
helper = { path = "patched/helper" }
"#,
        );
        write(
            &tmp.path().join("patched/helper/Cargo.toml"),
            "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&tmp.path().join("patched/helper/src/lib.rs"), "");
        let manifest = tmp.path().join("core/Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
helper = "1"
"#,
        );
        write(&tmp.path().join("core/src/lib.rs"), "");

        let out = synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path())
            .expect("synthesis succeeds");
        let root = read_toml(&out.root_manifest);
        let patch = root
            .get("patch")
            .and_then(|v| v.get("crates-io"))
            .and_then(|v| v.get("helper"))
            .and_then(toml::Value::as_table)
            .expect("carried patch");
        assert_eq!(
            patch.get("path").and_then(toml::Value::as_str),
            Some(
                tmp.path()
                    .join("patched/helper")
                    .to_str()
                    .expect("utf8 temp path")
            )
        );

        write(
            &ws_manifest,
            r#"
[workspace]
members = ["core"]

[patch.crates-io]
core = { path = "core" }
"#,
        );
        let err =
            synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path()).unwrap_err();
        assert!(config_invalid_message(err).contains("patch"));
    }

    #[test]
    fn member_local_patch_and_replace_reject() {
        let tmp = TempDir::new().expect("tempdir");
        let manifest = tmp.path().join("core/Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
helper = { path = "../helper" }
"#,
        );
        write(&tmp.path().join("core/src/lib.rs"), "");
        write(
            &tmp.path().join("helper/Cargo.toml"),
            r#"
[package]
name = "helper"
version = "0.1.0"
edition = "2021"

[patch.crates-io]
serde = { path = "../serde" }
"#,
        );
        write(&tmp.path().join("helper/src/lib.rs"), "");

        let err =
            synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path()).unwrap_err();
        assert!(config_invalid_message(err).contains("member-local [patch"));

        write(
            &tmp.path().join("helper/Cargo.toml"),
            r#"
[package]
name = "helper"
version = "0.1.0"
edition = "2021"

[replace]
"foo:1.0.0" = { path = "../foo" }
"#,
        );
        let err =
            synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path()).unwrap_err();
        assert!(config_invalid_message(err).contains("member-local [replace"));
    }

    #[test]
    fn selected_workspace_dependency_path_entry_is_staged_and_rewrite_aware() {
        let tmp = TempDir::new().expect("tempdir");
        write(
            &tmp.path().join("Cargo.toml"),
            r#"
[workspace]
members = ["core"]

[workspace.dependencies]
helper = { path = "helper" }
core = "1"
"#,
        );
        let manifest = tmp.path().join("core/Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
helper = { workspace = true }
"#,
        );
        write(&tmp.path().join("core/src/lib.rs"), "");
        write(
            &tmp.path().join("helper/Cargo.toml"),
            r#"
[package]
name = "helper"
version = "0.1.0"
edition = "2021"

[dependencies]
core = { path = "../core" }
"#,
        );
        write(&tmp.path().join("helper/src/lib.rs"), "");

        let out = synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path())
            .expect("synthesis succeeds");
        let root = read_toml(&out.root_manifest);
        let ws_helper = root
            .get("workspace")
            .and_then(|v| v.get("dependencies"))
            .and_then(|v| v.get("helper"))
            .and_then(toml::Value::as_table)
            .expect("workspace dependency helper");
        assert_eq!(
            ws_helper.get("path").and_then(toml::Value::as_str),
            Some("staged-path-deps/helper")
        );
        assert_eq!(
            root.get("workspace")
                .and_then(|v| v.get("dependencies"))
                .and_then(|v| v.get("core"))
                .and_then(toml::Value::as_str),
            Some("1"),
            "unused workspace dependency self-edges remain inert"
        );

        let staged_helper = out
            .staged_path_dep_manifests
            .iter()
            .find(|p| {
                table(&read_toml(p), "package")
                    .get("name")
                    .and_then(toml::Value::as_str)
                    == Some("helper")
            })
            .expect("helper staged");
        let helper = read_toml(staged_helper);
        assert_eq!(
            dep_table(&helper, "dependencies", "core")
                .get("path")
                .and_then(toml::Value::as_str),
            Some("../../dylib")
        );
    }

    #[test]
    fn workspace_package_and_replace_paths_are_absolutized() {
        let tmp = TempDir::new().expect("tempdir");
        write(&tmp.path().join("README.md"), "");
        write(&tmp.path().join("LICENSE"), "");
        write(
            &tmp.path().join("Cargo.toml"),
            r#"
[workspace]
members = ["core"]
resolver = "2"

[workspace.package]
readme = "README.md"
license-file = "LICENSE"

[replace]
"vendor:1.0.0" = { path = "vendor" }
"#,
        );
        let manifest = tmp.path().join("core/Cargo.toml");
        write(
            &manifest,
            r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
helper = "1"
"#,
        );
        write(&tmp.path().join("core/src/lib.rs"), "");
        write(
            &tmp.path().join("vendor/Cargo.toml"),
            "[package]\nname = \"vendor\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
        );

        let out = synthesize_workspace("core", &suite(&["helper"]), &manifest, tmp.path())
            .expect("synthesis succeeds");
        let root = read_toml(&out.root_manifest);
        let package = root
            .get("workspace")
            .and_then(|v| v.get("package"))
            .and_then(toml::Value::as_table)
            .expect("workspace package");
        assert_eq!(
            package.get("readme").and_then(toml::Value::as_str),
            Some(tmp.path().join("README.md").to_str().expect("utf8 path"))
        );
        assert_eq!(
            package.get("license-file").and_then(toml::Value::as_str),
            Some(tmp.path().join("LICENSE").to_str().expect("utf8 path"))
        );
        let replace = root
            .get("replace")
            .and_then(|v| v.get("vendor:1.0.0"))
            .and_then(toml::Value::as_table)
            .expect("replace entry");
        assert_eq!(
            replace.get("path").and_then(toml::Value::as_str),
            Some(tmp.path().join("vendor").to_str().expect("utf8 path"))
        );
    }

    #[test]
    fn offline_failure_note_mentions_staged_lockfile_and_remedies() {
        let tmp = TempDir::new().expect("tempdir");
        let root_manifest = tmp.path().join("lihaaf-suite-workspace/Cargo.toml");
        let source_lock = tmp.path().join("Cargo.lock");
        let plan = super::Synthesis {
            root_manifest,
            collector_manifest: tmp.path().join("collector/Cargo.toml"),
            staged_dylib_manifest: tmp.path().join("dylib/Cargo.toml"),
            staged_path_dep_manifests: Vec::new(),
            copied_lockfile: Some(source_lock.clone()),
        };
        let mut stderr =
            "error: no matching package named `serde` found in offline mode".to_string();

        super::append_offline_lockfile_note(&mut stderr, &plan);

        assert!(stderr.contains("copied the ancestor Cargo.lock"));
        assert!(stderr.contains(source_lock.to_str().expect("utf8 path")));
        assert!(stderr.contains("Warm Cargo"));
        assert!(stderr.contains("build_targets"));
    }
}
