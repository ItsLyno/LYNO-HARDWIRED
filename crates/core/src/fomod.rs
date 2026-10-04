//! FOMOD installers: `fomod/ModuleConfig.xml` in a mod archive asks the
//! player which options to install. Format of MO2's `installer_fomod` and
//! Vortex (schema 5.0, older files read the same):
//!
//! - `requiredInstallFiles` always go in;
//! - `installSteps` are pages of `optionalFileGroups`; a group says how many
//!   of its `plugins` may be picked (`SelectExactlyOne`, …);
//! - a picked plugin installs its `files` and sets `conditionFlags`; flags
//!   decide which later steps are `visible`, the type of a plugin
//!   (`dependencyType`: required, recommended, not usable…) and the
//!   `conditionalFileInstalls` at the end.
//!
//! The launcher evaluates everything here and the wizard in the UI only shows
//! the result, so the files installed are exactly what the wizard showed.
//!
//! A choice is remembered by step, group and plugin names in `meta.ini`
//! (`[LYNO] fomod`): the next version of the mod usually asks the same
//! questions, and the wizard starts from the old answers.

use std::collections::HashMap;

use base64::Engine;
use roxmltree::Node;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupKind {
    ExactlyOne,
    AtMostOne,
    AtLeastOne,
    All,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginKind {
    Required,
    Optional,
    Recommended,
    NotUsable,
    CouldBeUsable,
}

/// `fileDependency` states. Cyberpunk has no plugin load order: a file is
/// active when an enabled mod has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Active,
    Inactive,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileItem {
    /// Relative to the folder that holds `fomod/`, with `/`.
    pub source: String,
    /// `None`: the attribute is missing, the file keeps its source path.
    pub destination: Option<String>,
    pub folder: bool,
    pub priority: i64,
    /// Installed with its plugin picked or not.
    pub always: bool,
    /// Installed unless its plugin is `NotUsable`.
    pub if_usable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Flag { name: String, value: String },
    File { path: String, state: FileState },
    /// Game, script extender and manager versions: not checked.
    True,
}

impl Condition {
    fn eval(&self, flags: &HashMap<String, String>, files: &dyn Fn(&str) -> FileState) -> bool {
        match self {
            Condition::All(c) => c.iter().all(|c| c.eval(flags, files)),
            Condition::Any(c) => c.iter().any(|c| c.eval(flags, files)),
            // An unset flag reads as empty: `value=""` matches it.
            Condition::Flag { name, value } => flags.get(name).map_or("", String::as_str) == value,
            Condition::File { path, state } => files(path) == *state,
            Condition::True => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypeRule {
    default: PluginKind,
    patterns: Vec<(Condition, PluginKind)>,
}

impl TypeRule {
    fn resolve(&self, flags: &HashMap<String, String>, files: &dyn Fn(&str) -> FileState) -> PluginKind {
        self.patterns.iter().find(|(c, _)| c.eval(flags, files)).map_or(self.default, |(_, k)| *k)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    /// Path in the archive, relative to the folder that holds `fomod/`.
    pub image: Option<String>,
    files: Vec<FileItem>,
    flags: Vec<(String, String)>,
    kind: TypeRule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub kind: GroupKind,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    visible: Condition,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installer {
    pub name: Option<String>,
    pub image: Option<String>,
    required: Vec<FileItem>,
    pub steps: Vec<Step>,
    conditional: Vec<(Condition, Vec<FileItem>)>,
}

/// Picked plugins by step, group and plugin index. `None` for a step the
/// player hasn't seen: it gets the installer's defaults.
pub type Selection = Vec<Option<Vec<Vec<usize>>>>;

/// The wizard at a [`Selection`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evaluated {
    /// Per step: shown with the flags of the steps before it.
    pub visible: Vec<bool>,
    /// Per step, group and plugin.
    pub kinds: Vec<Vec<Vec<PluginKind>>>,
    /// The selection with defaults and rules applied; empty for hidden steps.
    pub selection: Vec<Vec<Vec<usize>>>,
    /// Per step: every group has an allowed number of picks.
    pub valid: Vec<bool>,
    #[serde(skip)]
    flags: HashMap<String, String>,
}

/// The installer as the wizard shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outline {
    pub name: Option<String>,
    pub image: Option<String>,
    pub steps: Vec<OutlineStep>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineStep {
    pub name: String,
    pub groups: Vec<OutlineGroup>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineGroup {
    pub name: String,
    pub kind: GroupKind,
    pub plugins: Vec<OutlinePlugin>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlinePlugin {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
}

/// A remembered choice, by names: indices change between versions of a mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedStep {
    pub step: String,
    pub groups: Vec<SavedGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedGroup {
    pub group: String,
    pub plugins: Vec<String>,
}

impl Installer {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text = decode(bytes);
        let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(parse_error)?;
        let root = doc.root_element();
        if !root.tag_name().name().eq_ignore_ascii_case("config") {
            return Err(parse_error(format!("root element is <{}>, not <config>", root.tag_name().name())));
        }
        let mut steps: Vec<Step> = match child(root, "installSteps") {
            Some(s) => ordered(s, children(s, "installStep").map(parse_step).collect::<Result<Vec<_>>>()?, |s: &Step| s.name.as_str()),
            None => Vec::new(),
        };
        steps.retain(|s| !s.groups.is_empty());
        let conditional = match child(root, "conditionalFileInstalls").and_then(|c| child(c, "patterns")) {
            Some(p) => children(p, "pattern")
                .map(|p| Ok((child(p, "dependencies").map_or(Condition::True, composite), files(child(p, "files"))?)))
                .collect::<Result<_>>()?,
            None => Vec::new(),
        };
        Ok(Self {
            name: child(root, "moduleName").and_then(|n| n.text()).map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()),
            image: child(root, "moduleImage").and_then(|n| attr(n, "path")).map(normalize).filter(|p| !p.is_empty()),
            required: files(child(root, "requiredInstallFiles"))?,
            steps,
            conditional,
        })
    }

    pub fn outline(&self) -> Outline {
        Outline {
            name: self.name.clone(),
            image: self.image.clone(),
            steps: self
                .steps
                .iter()
                .map(|s| OutlineStep {
                    name: s.name.clone(),
                    groups: s
                        .groups
                        .iter()
                        .map(|g| OutlineGroup {
                            name: g.name.clone(),
                            kind: g.kind,
                            plugins: g
                                .plugins
                                .iter()
                                .map(|p| OutlinePlugin { name: p.name.clone(), description: p.description.clone(), image: p.image.clone() })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Every image the wizard may show.
    pub fn images(&self) -> Vec<String> {
        let mut out: Vec<String> = self.image.iter().cloned().collect();
        for p in self.steps.iter().flat_map(|s| &s.groups).flat_map(|g| &g.plugins) {
            out.extend(p.image.clone());
        }
        out.sort();
        out.dedup();
        out
    }

    /// Walks the steps in order: each sees the flags of the visible steps
    /// before it, like the wizard pages one after another.
    pub fn evaluate(&self, selection: &Selection, files: &dyn Fn(&str) -> FileState) -> Evaluated {
        let mut flags = HashMap::new();
        let mut ev = Evaluated { visible: vec![], kinds: vec![], selection: vec![], valid: vec![], flags: HashMap::new() };
        for (i, step) in self.steps.iter().enumerate() {
            let visible = step.visible.eval(&flags, files);
            let kinds: Vec<Vec<PluginKind>> =
                step.groups.iter().map(|g| g.plugins.iter().map(|p| p.kind.resolve(&flags, files)).collect()).collect();
            let (picked, valid) = if visible {
                let given = selection.get(i).and_then(Option::as_ref);
                let picked: Vec<Vec<usize>> = step
                    .groups
                    .iter()
                    .zip(&kinds)
                    .enumerate()
                    .map(|(g, (group, kinds))| pick(group.kind, kinds, given.and_then(|s| s.get(g)).map(Vec::as_slice)))
                    .collect();
                for (group, picked) in step.groups.iter().zip(&picked) {
                    for &p in picked {
                        for (name, value) in &group.plugins[p].flags {
                            flags.insert(name.clone(), value.clone());
                        }
                    }
                }
                let valid = step.groups.iter().zip(&kinds).zip(&picked).all(|((g, k), p)| allowed(g.kind, k, p.len()));
                (picked, valid)
            } else {
                (Vec::new(), true)
            };
            ev.visible.push(visible);
            ev.kinds.push(kinds);
            ev.selection.push(picked);
            ev.valid.push(valid);
        }
        ev.flags = flags;
        ev
    }

    /// What to install for an evaluated selection, lowest priority first: a
    /// later item overwrites an earlier one with the same destination.
    pub fn files(&self, ev: &Evaluated, files: &dyn Fn(&str) -> FileState) -> Vec<FileItem> {
        let mut out = self.required.clone();
        for (i, step) in self.steps.iter().enumerate() {
            if !ev.visible.get(i).copied().unwrap_or(false) {
                continue;
            }
            for (g, group) in step.groups.iter().enumerate() {
                for (p, plugin) in group.plugins.iter().enumerate() {
                    let picked = ev.selection[i].get(g).is_some_and(|s| s.contains(&p));
                    let usable = ev.kinds[i][g][p] != PluginKind::NotUsable;
                    out.extend(plugin.files.iter().filter(|f| picked || f.always || (f.if_usable && usable)).cloned());
                }
            }
        }
        for (cond, items) in &self.conditional {
            if cond.eval(&ev.flags, files) {
                out.extend(items.iter().cloned());
            }
        }
        // Stable: equal priorities keep the order of the file.
        out.sort_by_key(|f| f.priority);
        out
    }

    pub fn save(&self, ev: &Evaluated) -> Vec<SavedStep> {
        self.steps
            .iter()
            .enumerate()
            .filter(|(i, _)| ev.visible[*i])
            .map(|(i, s)| SavedStep {
                step: s.name.clone(),
                groups: s
                    .groups
                    .iter()
                    .zip(&ev.selection[i])
                    .map(|(g, picked)| SavedGroup { group: g.name.clone(), plugins: picked.iter().map(|&p| g.plugins[p].name.clone()).collect() })
                    .collect(),
            })
            .collect()
    }

    /// A remembered choice on this installer. Steps and groups are matched by
    /// name in order; a step that isn't remembered gets the defaults.
    pub fn restore(&self, saved: &[SavedStep]) -> Selection {
        let mut used = vec![false; saved.len()];
        self.steps
            .iter()
            .map(|step| {
                let i = saved.iter().enumerate().position(|(i, s)| !used[i] && s.step == step.name)?;
                used[i] = true;
                let mut groups_used = vec![false; saved[i].groups.len()];
                Some(
                    step.groups
                        .iter()
                        .map(|g| {
                            let Some(j) = saved[i].groups.iter().enumerate().position(|(j, s)| !groups_used[j] && s.group == g.name) else {
                                return Vec::new();
                            };
                            groups_used[j] = true;
                            let names = &saved[i].groups[j].plugins;
                            (0..g.plugins.len()).filter(|&p| names.contains(&g.plugins[p].name)).collect()
                        })
                        .collect(),
                )
            })
            .collect()
    }
}

/// For `meta.ini`: MO2 rewrites the file through QSettings, which splits an
/// unquoted value at commas, so the JSON goes in as base64.
pub fn encode_saved(saved: &[SavedStep]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(saved).unwrap_or_default())
}

pub fn decode_saved(value: &str) -> Option<Vec<SavedStep>> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value.trim()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Picked plugins of a group: the player's picks (`None`: not seen yet, the
/// defaults) within the group's rules and the plugins' types.
fn pick(kind: GroupKind, kinds: &[PluginKind], given: Option<&[usize]>) -> Vec<usize> {
    let usable = |p: &usize| kinds[*p] != PluginKind::NotUsable;
    let all: Vec<usize> = (0..kinds.len()).filter(usable).collect();
    if kind == GroupKind::All {
        return all;
    }
    let required: Vec<usize> = all.iter().copied().filter(|&p| kinds[p] == PluginKind::Required).collect();
    let mut picked: Vec<usize> = match given {
        Some(g) => g.iter().copied().filter(|&p| p < kinds.len() && usable(&p)).collect(),
        None => all.iter().copied().filter(|&p| kinds[p] == PluginKind::Recommended).collect(),
    };
    picked.extend(&required);
    picked.sort_unstable();
    picked.dedup();
    match kind {
        GroupKind::ExactlyOne | GroupKind::AtMostOne if picked.len() > 1 => {
            // A required plugin wins over the player's pick.
            let keep = required.first().or_else(|| picked.first()).copied();
            picked = keep.into_iter().collect();
        }
        GroupKind::ExactlyOne | GroupKind::AtLeastOne if picked.is_empty() && given.is_none() => {
            picked.extend(all.first());
        }
        _ => {}
    }
    picked
}

fn allowed(kind: GroupKind, kinds: &[PluginKind], picked: usize) -> bool {
    // A group whose every plugin is unusable can't be satisfied: not the player's fault.
    let any_usable = kinds.iter().any(|k| *k != PluginKind::NotUsable);
    match kind {
        GroupKind::ExactlyOne => picked == 1 || !any_usable,
        GroupKind::AtLeastOne => picked >= 1 || !any_usable,
        GroupKind::AtMostOne => picked <= 1,
        GroupKind::All | GroupKind::Any => true,
    }
}

/// ModuleConfig.xml comes as UTF-8 or UTF-16 (often with a BOM; Notepad and
/// FOMOD Creator write both).
fn decode(bytes: &[u8]) -> String {
    let utf16 = |bytes: &[u8], le: bool| {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [b'<', 0, ..] => utf16(bytes, true),
        [0, b'<', ..] => utf16(bytes, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn parse_error(e: impl std::fmt::Display) -> Error {
    Error::Parse { path: "fomod/ModuleConfig.xml".into(), message: e.to_string() }
}

/// Element names are matched without case: hand-written installers vary.
fn children<'a, 'i: 'a>(n: Node<'a, 'i>, tag: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children().filter(move |c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(tag))
}

fn child<'a, 'i: 'a>(n: Node<'a, 'i>, tag: &'static str) -> Option<Node<'a, 'i>> {
    children(n, tag).next()
}

fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name().eq_ignore_ascii_case(name)).map(|a| a.value())
}

fn normalize(path: &str) -> String {
    path.trim().replace('\\', "/").trim_start_matches("./").trim_matches('/').to_owned()
}

/// `order` of steps, groups and plugins: alphabetical unless `Explicit`.
fn ordered<T>(n: Node, mut items: Vec<T>, name: impl Fn(&T) -> &str) -> Vec<T> {
    match attr(n, "order").unwrap_or("Ascending") {
        o if o.eq_ignore_ascii_case("Explicit") => {}
        o if o.eq_ignore_ascii_case("Descending") => {
            items.sort_by_key(|i| std::cmp::Reverse(name(i).to_lowercase()));
        }
        _ => items.sort_by_key(|i| name(i).to_lowercase()),
    }
    items
}

fn parse_step(n: Node) -> Result<Step> {
    let groups = match child(n, "optionalFileGroups") {
        Some(g) => ordered(g, children(g, "group").map(parse_group).collect::<Result<Vec<_>>>()?, |g: &Group| g.name.as_str()),
        None => Vec::new(),
    };
    Ok(Step {
        name: attr(n, "name").unwrap_or_default().to_owned(),
        visible: child(n, "visible").map_or(Condition::True, composite),
        groups: groups.into_iter().filter(|g| !g.plugins.is_empty()).collect(),
    })
}

fn parse_group(n: Node) -> Result<Group> {
    let kind = match attr(n, "type").unwrap_or("SelectAny").to_ascii_lowercase().as_str() {
        "selectexactlyone" => GroupKind::ExactlyOne,
        "selectatmostone" => GroupKind::AtMostOne,
        "selectatleastone" => GroupKind::AtLeastOne,
        "selectall" => GroupKind::All,
        _ => GroupKind::Any,
    };
    let plugins = match child(n, "plugins") {
        Some(p) => ordered(p, children(p, "plugin").map(parse_plugin).collect::<Result<Vec<_>>>()?, |p: &Plugin| p.name.as_str()),
        None => Vec::new(),
    };
    Ok(Group { name: attr(n, "name").unwrap_or_default().to_owned(), kind, plugins })
}

fn parse_plugin(n: Node) -> Result<Plugin> {
    let flags = child(n, "conditionFlags")
        .map(|f| {
            children(f, "flag")
                .filter_map(|f| Some((attr(f, "name")?.to_owned(), f.text().unwrap_or_default().trim().to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let kind = match child(n, "typeDescriptor") {
        Some(t) => match (child(t, "type"), child(t, "dependencyType")) {
            (Some(t), _) => TypeRule { default: plugin_kind(t), patterns: vec![] },
            (None, Some(d)) => TypeRule {
                default: child(d, "defaultType").map_or(PluginKind::Optional, plugin_kind),
                patterns: child(d, "patterns")
                    .map(|p| {
                        children(p, "pattern")
                            .map(|p| {
                                let cond = child(p, "dependencies").map_or(Condition::True, composite);
                                (cond, child(p, "type").map_or(PluginKind::Optional, plugin_kind))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            (None, None) => TypeRule { default: PluginKind::Optional, patterns: vec![] },
        },
        None => TypeRule { default: PluginKind::Optional, patterns: vec![] },
    };
    Ok(Plugin {
        name: attr(n, "name").unwrap_or_default().to_owned(),
        description: child(n, "description").and_then(|d| d.text()).unwrap_or_default().trim().to_owned(),
        image: child(n, "image").and_then(|i| attr(i, "path")).map(normalize).filter(|p| !p.is_empty()),
        files: files(child(n, "files"))?,
        flags,
        kind,
    })
}

fn plugin_kind(n: Node) -> PluginKind {
    match attr(n, "name").unwrap_or_default().to_ascii_lowercase().as_str() {
        "required" => PluginKind::Required,
        "recommended" => PluginKind::Recommended,
        "notusable" => PluginKind::NotUsable,
        "couldbeusable" => PluginKind::CouldBeUsable,
        _ => PluginKind::Optional,
    }
}

/// `<dependencies operator="And|Or">` (or `<visible>`, the same type) with
/// flag, file, game and nested dependencies.
fn composite(n: Node) -> Condition {
    let items: Vec<Condition> = n
        .children()
        .filter(Node::is_element)
        .map(|c| match c.tag_name().name().to_ascii_lowercase().as_str() {
            "flagdependency" => Condition::Flag {
                name: attr(c, "flag").unwrap_or_default().to_owned(),
                value: attr(c, "value").unwrap_or_default().to_owned(),
            },
            "filedependency" => Condition::File {
                path: normalize(attr(c, "file").unwrap_or_default()),
                state: match attr(c, "state").unwrap_or_default().to_ascii_lowercase().as_str() {
                    "active" => FileState::Active,
                    "inactive" => FileState::Inactive,
                    _ => FileState::Missing,
                },
            },
            "dependencies" => composite(c),
            _ => Condition::True,
        })
        .collect();
    if attr(n, "operator").is_some_and(|o| o.eq_ignore_ascii_case("Or")) {
        Condition::Any(items)
    } else {
        Condition::All(items)
    }
}

fn files(n: Option<Node>) -> Result<Vec<FileItem>> {
    let Some(n) = n else { return Ok(Vec::new()) };
    let flag = |n: Node, name: &str| attr(n, name).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    n.children()
        .filter(Node::is_element)
        .filter_map(|c| {
            let folder = match c.tag_name().name().to_ascii_lowercase().as_str() {
                "file" => false,
                "folder" => true,
                _ => return None,
            };
            Some(match attr(c, "source") {
                None => Err(parse_error(format!("<{}> without source", c.tag_name().name()))),
                Some(source) => Ok(FileItem {
                    source: normalize(source),
                    destination: attr(c, "destination").map(normalize),
                    folder,
                    priority: attr(c, "priority").and_then(|p| p.trim().parse().ok()).unwrap_or(0),
                    always: flag(c, "alwaysInstall"),
                    if_usable: flag(c, "installIfUsable"),
                }),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <moduleName>Cool Mod</moduleName>
  <moduleImage path="fomod\images\main.png" />
  <requiredInstallFiles>
    <folder source="Core" destination="" />
  </requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Variant">
      <optionalFileGroups order="Explicit">
        <group name="Body" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="Vanilla">
              <description> Plain body </description>
              <image path="fomod/images/vanilla.png" />
              <files><folder source="Vanilla" destination="" /></files>
              <conditionFlags><flag name="body">vanilla</flag></conditionFlags>
              <typeDescriptor><type name="Optional" /></typeDescriptor>
            </plugin>
            <plugin name="Athletic">
              <description>Fit</description>
              <files><file source="Athletic\body.archive" destination="archive\pc\mod\body.archive" priority="1" /></files>
              <conditionFlags><flag name="body">athletic</flag></conditionFlags>
              <typeDescriptor><type name="Recommended" /></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
    <installStep name="Athletic extras">
      <visible><flagDependency flag="body" value="athletic" /></visible>
      <optionalFileGroups>
        <group name="Extras" type="SelectAny">
          <plugins>
            <plugin name="Tattoos">
              <description />
              <files><file source="Extras/tattoo.archive" /></files>
              <typeDescriptor>
                <dependencyType>
                  <defaultType name="Optional" />
                  <patterns>
                    <pattern>
                      <dependencies operator="And"><fileDependency file="archive/pc/mod/base.archive" state="Missing" /></dependencies>
                      <type name="NotUsable" />
                    </pattern>
                  </patterns>
                </dependencyType>
              </typeDescriptor>
            </plugin>
            <plugin name="A Scars">
              <files><file source="Extras/scars.archive" alwaysInstall="true" /></files>
              <typeDescriptor><type name="Optional" /></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
  <conditionalFileInstalls>
    <patterns>
      <pattern>
        <dependencies operator="Or"><flagDependency flag="body" value="vanilla" /></dependencies>
        <files><file source="Patches/vanilla.xl" destination="archive/pc/mod/vanilla.xl" /></files>
      </pattern>
    </patterns>
  </conditionalFileInstalls>
</config>"#;

    fn missing(_: &str) -> FileState {
        FileState::Missing
    }

    fn active(_: &str) -> FileState {
        FileState::Active
    }

    fn sources(items: &[FileItem]) -> Vec<&str> {
        items.iter().map(|f| f.source.as_str()).collect()
    }

    #[test]
    fn parses_the_installer() {
        let i = Installer::parse(XML.as_bytes()).unwrap();
        assert_eq!(i.name.as_deref(), Some("Cool Mod"));
        assert_eq!(i.image.as_deref(), Some("fomod/images/main.png"));
        assert_eq!(i.steps.len(), 2);
        let body = &i.steps[0].groups[0];
        assert_eq!(body.kind, GroupKind::ExactlyOne);
        assert_eq!(body.plugins[0].description, "Plain body");
        assert_eq!(body.plugins[1].files[0].destination.as_deref(), Some("archive/pc/mod/body.archive"));
        // Ascending by default: "A Scars" before "Tattoos".
        let extras: Vec<&str> = i.steps[1].groups[0].plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(extras, ["A Scars", "Tattoos"]);
        assert_eq!(i.images(), ["fomod/images/main.png", "fomod/images/vanilla.png"]);
    }

    #[test]
    fn defaults_follow_types_and_flags_show_steps() {
        let i = Installer::parse(XML.as_bytes()).unwrap();
        // Recommended "Athletic" is the default; its flag shows the second step.
        let ev = i.evaluate(&vec![None, None], &missing);
        assert_eq!(ev.selection[0], [vec![1]]);
        assert_eq!(ev.visible, [true, true]);
        // The tattoo needs a base file no mod has.
        assert_eq!(ev.kinds[1][0], [PluginKind::Optional, PluginKind::NotUsable]);
        assert_eq!(ev.selection[1], [Vec::<usize>::new()]);
        assert_eq!(ev.valid, [true, true]);
        let files = i.files(&ev, &missing);
        // Required first, `alwaysInstall` without a pick, priority 1 last.
        assert_eq!(sources(&files), ["Core", "Extras/scars.archive", "Athletic/body.archive"]);

        let tattoo = i.evaluate(&vec![None, Some(vec![vec![1]])], &active);
        assert_eq!(tattoo.selection[1], [vec![1]]);
        assert!(sources(&i.files(&tattoo, &active)).contains(&"Extras/tattoo.archive"));
        let not_usable = i.evaluate(&vec![None, Some(vec![vec![1]])], &missing);
        assert_eq!(not_usable.selection[1], [Vec::<usize>::new()], "a NotUsable pick is dropped");
    }

    #[test]
    fn hidden_steps_and_conditional_files() {
        let i = Installer::parse(XML.as_bytes()).unwrap();
        let ev = i.evaluate(&vec![Some(vec![vec![0]]), Some(vec![vec![0, 1]])], &active);
        assert_eq!(ev.visible, [true, false]);
        assert!(ev.selection[1].is_empty());
        assert_eq!(sources(&i.files(&ev, &active)), ["Core", "Vanilla", "Patches/vanilla.xl"]);
    }

    #[test]
    fn group_rules() {
        use PluginKind::*;
        assert_eq!(pick(GroupKind::ExactlyOne, &[Optional, Optional], None), [0]);
        assert_eq!(pick(GroupKind::ExactlyOne, &[Optional, Required], Some(&[0])), [1]);
        assert_eq!(pick(GroupKind::ExactlyOne, &[NotUsable, Optional], None), [1]);
        assert_eq!(pick(GroupKind::AtMostOne, &[Recommended, Recommended], None), [0]);
        assert_eq!(pick(GroupKind::All, &[Optional, NotUsable, Optional], Some(&[])), [0, 2]);
        assert_eq!(pick(GroupKind::Any, &[Optional, Optional], Some(&[1, 5])), [1]);
        // The player unpicked the only option of an ExactlyOne group: not valid, not re-picked.
        assert!(pick(GroupKind::ExactlyOne, &[Optional, Optional], Some(&[])).is_empty());
        assert!(!allowed(GroupKind::ExactlyOne, &[Optional], 0));
        assert!(allowed(GroupKind::ExactlyOne, &[NotUsable], 0));
        assert!(!allowed(GroupKind::AtMostOne, &[Optional, Optional], 2));
    }

    #[test]
    fn remembers_choices_by_name() {
        let i = Installer::parse(XML.as_bytes()).unwrap();
        let ev = i.evaluate(&vec![Some(vec![vec![1]]), Some(vec![vec![0]])], &missing);
        let saved = i.save(&ev);
        assert_eq!(saved[1], SavedStep { step: "Athletic extras".into(), groups: vec![SavedGroup { group: "Extras".into(), plugins: vec!["A Scars".into()] }] });
        let value = encode_saved(&saved);
        assert!(!value.contains(',') && !value.contains('='), "{value}");
        let back = decode_saved(&value).unwrap();
        assert_eq!(i.restore(&back), vec![Some(vec![vec![1]]), Some(vec![vec![0]])]);
        // A new version renamed the second step: it starts from the defaults.
        let renamed = vec![saved[0].clone(), SavedStep { step: "Other".into(), groups: vec![] }];
        assert_eq!(i.restore(&renamed), vec![Some(vec![vec![1]]), None]);
        assert!(decode_saved("not base64!").is_none());
    }

    #[test]
    fn reads_utf16() {
        let mut bytes = vec![0xFF, 0xFE];
        for u in XML.replace("utf-8", "utf-16").encode_utf16() {
            bytes.extend(u.to_le_bytes());
        }
        assert_eq!(Installer::parse(&bytes).unwrap().steps.len(), 2);
        let bom8 = [&[0xEF, 0xBB, 0xBF][..], XML.as_bytes()].concat();
        assert!(Installer::parse(&bom8).is_ok());
        assert!(Installer::parse(b"<nope/>").is_err());
        assert!(Installer::parse(b"garbage").is_err());
    }

    #[test]
    fn visible_with_nested_dependencies() {
        let xml = r#"<config><installSteps><installStep name="s"><visible><dependencies operator="Or">
            <flagDependency flag="a" value="1"/><flagDependency flag="b" value=""/></dependencies></visible>
            <optionalFileGroups><group name="g" type="SelectAll"><plugins><plugin name="p"><description/>
            <files><file source="x"/></files><typeDescriptor><type name="Required"/></typeDescriptor></plugin></plugins></group>
            </optionalFileGroups></installStep></installSteps></config>"#;
        let i = Installer::parse(xml.as_bytes()).unwrap();
        // Flag b is unset, which reads as "".
        assert_eq!(i.evaluate(&vec![None], &missing).visible, [true]);
    }
}
