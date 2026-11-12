//! Dockable panels and workspaces (ADR 0075): where each panel is, as data.
//!
//! The editor's panels live in four docks around the stage (left, right, top and bottom). A
//! dock holds tab groups: stacked top to bottom in the side docks, side by side in the top and
//! bottom ones. A group shows one of its panels at a time, as tabs. A panel can also float in a
//! window of its own, or be closed (Window menu reopens it where it was).
//!
//! A workspace is a named layout: three built in (Essentials, today's layout; Animator; Designer)
//! and the user's own (Window › Workspace › Save Workspace…). Each keeps the changes made to it
//! until Reset, as in Animate. Everything here is pure, so it is tested without a window; the
//! shell draws it (`dock_ui`). It is view state, kept in the preferences, never in a document.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

/// A panel that can be docked, stacked, floated and closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Panel {
    Properties,
    Align,
    Colour,
    Library,
    MovieExplorer,
    History,
    Timeline,
    MotionEditor,
    Actions,
    Output,
}

impl Panel {
    /// In the Window menu's order.
    pub const ALL: [Panel; 10] = [Panel::Actions, Panel::Align, Panel::Colour, Panel::History, Panel::Library, Panel::MotionEditor, Panel::MovieExplorer, Panel::Output, Panel::Properties, Panel::Timeline];

    /// Its machine name, in the preferences (English, never translated).
    pub fn id(self) -> &'static str {
        match self {
            Panel::Properties => "properties",
            Panel::Align => "align",
            Panel::Colour => "colour",
            Panel::Library => "library",
            Panel::MovieExplorer => "movie_explorer",
            Panel::History => "history",
            Panel::Timeline => "timeline",
            Panel::MotionEditor => "motion_editor",
            Panel::Actions => "actions",
            Panel::Output => "output",
        }
    }

    pub fn from_id(id: &str) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Its name (English: translate where it is shown with `crate::i18n::t`).
    pub fn label(self) -> &'static str {
        match self {
            Panel::Properties => tr_noop!("Properties"),
            Panel::Align => tr_noop!("Align"),
            Panel::Colour => tr_noop!("Colour"),
            Panel::Library => tr_noop!("Library"),
            Panel::MovieExplorer => tr_noop!("Movie Explorer"),
            Panel::History => tr_noop!("History"),
            Panel::Timeline => tr_noop!("Timeline"),
            Panel::MotionEditor => tr_noop!("Motion Editor"),
            Panel::Actions => tr_noop!("Actions"),
            Panel::Output => tr_noop!("Output"),
        }
    }

    /// The size a floating window of it starts at, in points.
    pub fn float_size(self) -> [f32; 2] {
        match self {
            Panel::Timeline | Panel::MotionEditor => [900.0, 320.0],
            Panel::Actions => [460.0, 520.0],
            Panel::Output => [520.0, 300.0],
            Panel::Align => [300.0, 140.0],
            _ => [320.0, 460.0],
        }
    }
}

impl Serialize for Panel {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for Panel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Panel, D::Error> {
        let id = String::deserialize(d)?;
        // i18n-exempt: a preferences-file parse error, never shown (the entry is skipped)
        Panel::from_id(&id).ok_or_else(|| serde::de::Error::custom(format!("unknown panel {id}")))
    }
}

/// A dock around the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

    /// Groups stack top to bottom (left and right docks) rather than side by side.
    pub fn stacks(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }

    /// English: translate where it is shown.
    pub fn label(self) -> &'static str {
        match self {
            Side::Left => tr_noop!("Dock Left"),
            Side::Right => tr_noop!("Dock Right"),
            Side::Top => tr_noop!("Dock Above the Stage"),
            Side::Bottom => tr_noop!("Dock Below the Stage"),
        }
    }
}

/// Panels shown as tabs in one place; one of them at a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Group {
    #[serde(deserialize_with = "lenient_vec")]
    pub tabs: Vec<Panel>,
    /// The tab shown.
    pub active: usize,
    /// Its part of the dock's length, against the other groups' (only the ratio counts).
    pub share: f32,
    /// Folded to its header (side docks only).
    pub folded: bool,
}

impl Default for Group {
    fn default() -> Group {
        Group { tabs: Vec::new(), active: 0, share: 1.0, folded: false }
    }
}

impl Group {
    fn of(tabs: &[Panel], share: f32) -> Group {
        Group { tabs: tabs.to_vec(), share, ..Group::default() }
    }

    /// The panel shown.
    pub fn shown(&self) -> Option<Panel> {
        self.tabs.get(self.active).or(self.tabs.first()).copied()
    }
}

/// One dock: its groups, and its width (side docks) or height (top and bottom).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Zone {
    #[serde(deserialize_with = "lenient_vec")]
    pub groups: Vec<Group>,
    /// In points; `None`: worked out from the window (the Essentials sizes).
    pub size: Option<f32>,
}

/// A panel floating in a window of its own (or inside the editor where there can't be one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Float {
    pub panel: Panel,
    /// Top-left corner, in points (on the desktop for a window of its own); `None`: let the
    /// system place it.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    pub size: [f32; 2],
}

/// Where a closed or floating panel goes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    pub side: Side,
    /// A panel it was stacked with or next to: it goes back into that one's group.
    #[serde(default, deserialize_with = "lenient_option")]
    pub near: Option<Panel>,
    /// It was floating when closed: it reopens floating.
    #[serde(default)]
    pub floating: bool,
}

/// Where a panel is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Docked { side: Side, group: usize, tab: usize },
    Floating(usize),
}

/// Where a moved panel goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// A tab in the group that holds this panel.
    Stack(Panel),
    /// A group of its own in `side`'s dock, before the group holding `before` (at the end when
    /// `None`, or when that panel isn't in that dock).
    Split { side: Side, before: Option<Panel> },
    /// A window of its own, its top-left corner here (`None`: where the system puts it).
    Float(Option<[f32; 2]>),
}

/// Bounds a layout read from a file is held to.
pub const MIN_ZONE: f32 = 80.0;
pub const MAX_ZONE: f32 = 4000.0;
pub const MIN_FLOAT: f32 = 160.0;

/// The whole arrangement.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub left: Zone,
    pub right: Zone,
    pub top: Zone,
    pub bottom: Zone,
    #[serde(deserialize_with = "lenient_vec")]
    pub floating: Vec<Float>,
    #[serde(deserialize_with = "lenient_map")]
    pub homes: BTreeMap<Panel, Home>,
}

impl Layout {
    pub fn zone(&self, side: Side) -> &Zone {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
            Side::Top => &self.top,
            Side::Bottom => &self.bottom,
        }
    }

    pub fn zone_mut(&mut self, side: Side) -> &mut Zone {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
            Side::Top => &mut self.top,
            Side::Bottom => &mut self.bottom,
        }
    }

    pub fn place(&self, p: Panel) -> Option<Place> {
        for side in Side::ALL {
            for (g, group) in self.zone(side).groups.iter().enumerate() {
                if let Some(tab) = group.tabs.iter().position(|t| *t == p) {
                    return Some(Place::Docked { side, group: g, tab });
                }
            }
        }
        self.floating.iter().position(|f| f.panel == p).map(Place::Floating)
    }

    pub fn is_open(&self, p: Panel) -> bool {
        self.place(p).is_some()
    }

    /// Open and in front: the shown tab of its group, or floating.
    pub fn is_shown(&self, p: Panel) -> bool {
        match self.place(p) {
            Some(Place::Docked { side, group, .. }) => self.zone(side).groups.get(group).and_then(Group::shown) == Some(p),
            Some(Place::Floating(_)) => true,
            None => false,
        }
    }

    pub fn is_floating(&self, p: Panel) -> bool {
        matches!(self.place(p), Some(Place::Floating(_)))
    }

    /// Every docked panel in Ctrl+F6 order: the top dock, the bottom one, the right, the left;
    /// then the floating ones.
    pub fn order(&self) -> Vec<Panel> {
        let mut v = Vec::new();
        for side in [Side::Top, Side::Bottom, Side::Right, Side::Left] {
            for g in &self.zone(side).groups {
                v.extend(g.tabs.iter().copied());
            }
        }
        v.extend(self.floating.iter().map(|f| f.panel));
        v
    }

    /// Takes `p` out of wherever it is, keeping where it was (its home). Empty groups go.
    fn take(&mut self, p: Panel) {
        match self.place(p) {
            Some(Place::Docked { side, group, tab }) => {
                let zone = self.zone_mut(side);
                let Some(g) = zone.groups.get_mut(group) else { return };
                g.tabs.remove(tab);
                let near = g.tabs.first().copied();
                if g.active >= tab && g.active > 0 {
                    g.active -= 1;
                }
                let near = near.or_else(|| {
                    // Its group goes: remember the group after it (or before, at the end).
                    let after = zone.groups.get(group + 1).and_then(|g| g.tabs.first().copied());
                    after.or_else(|| group.checked_sub(1).and_then(|i| zone.groups.get(i)).and_then(|g| g.tabs.first().copied()))
                });
                zone.groups.retain(|g| !g.tabs.is_empty());
                self.homes.insert(p, Home { side, near, floating: false });
            }
            Some(Place::Floating(i)) => {
                self.floating.remove(i);
                self.homes.entry(p).or_insert_with(|| default_home(p)).floating = true;
            }
            None => {}
        }
    }

    /// Closes `p` (Window menu, the tab's menu): it reopens where it was.
    pub fn close(&mut self, p: Panel) {
        self.take(p);
    }

    /// Brings `p` to the front: its tab shown, or reopened where it was when closed.
    pub fn show(&mut self, p: Panel) {
        match self.place(p) {
            Some(Place::Docked { side, group, tab }) => {
                if let Some(g) = self.zone_mut(side).groups.get_mut(group) {
                    g.active = tab;
                    g.folded = false;
                }
            }
            Some(Place::Floating(_)) => {}
            None => {
                let home = self.homes.get(&p).copied().unwrap_or_else(|| default_home(p));
                if home.floating {
                    self.move_to(p, Target::Float(None));
                } else {
                    self.dock_home(p, home);
                }
            }
        }
    }

    /// The Window menu's item: closes a panel in front, brings any other forward.
    pub fn toggle(&mut self, p: Panel) {
        if self.is_shown(p) {
            self.close(p);
        } else {
            self.show(p);
        }
    }

    /// Puts a floating panel back in the dock it came from.
    pub fn dock_back(&mut self, p: Panel) {
        if !self.is_floating(p) {
            return;
        }
        self.take(p);
        let home = self.homes.get(&p).copied().unwrap_or_else(|| default_home(p));
        self.dock_home(p, Home { floating: false, ..home });
    }

    fn dock_home(&mut self, p: Panel, home: Home) {
        let target = match home.near.filter(|n| *n != p && matches!(self.place(*n), Some(Place::Docked { side, .. }) if side == home.side)) {
            Some(n) => Target::Stack(n),
            None => Target::Split { side: home.side, before: None },
        };
        self.move_to(p, target);
    }

    /// Moves `p` (dragged by its tab, or from its menu). Moving onto itself changes nothing.
    pub fn move_to(&mut self, p: Panel, target: Target) {
        match target {
            Target::Stack(with) => {
                if with == p || !matches!(self.place(with), Some(Place::Docked { .. })) {
                    return;
                }
                self.take(p);
                if let Some(Place::Docked { side, group, .. }) = self.place(with)
                    && let Some(g) = self.zone_mut(side).groups.get_mut(group)
                {
                    g.tabs.push(p);
                    g.active = g.tabs.len() - 1;
                    g.folded = false;
                }
            }
            Target::Split { side, before } => {
                // Already alone in a group just there: nothing to do.
                if let Some(Place::Docked { side: s, group, .. }) = self.place(p)
                    && s == side
                    && self.zone(side).groups.get(group).is_some_and(|g| g.tabs.len() == 1)
                {
                    let next = self.zone(side).groups.get(group + 1).and_then(|g| g.tabs.first().copied());
                    if next == before {
                        return;
                    }
                }
                self.take(p);
                let zone = self.zone_mut(side);
                let at = before.and_then(|b| zone.groups.iter().position(|g| g.tabs.contains(&b))).unwrap_or(zone.groups.len());
                // A new group gets an even part of the dock.
                let share = if zone.groups.is_empty() { 1.0 } else { zone.groups.iter().map(|g| g.share).sum::<f32>() / zone.groups.len() as f32 };
                zone.groups.insert(at, Group::of(&[p], share));
            }
            Target::Float(at) => {
                if let Some(Place::Floating(i)) = self.place(p) {
                    if let (Some(f), Some(at)) = (self.floating.get_mut(i), at) {
                        f.at = Some(at);
                    }
                    return;
                }
                self.take(p);
                self.floating.push(Float { panel: p, at, size: p.float_size() });
            }
        }
    }

    /// Holds a layout read from a file to what the editor can draw: each panel once, no empty
    /// groups, finite sizes in range, the shown tab in range.
    pub fn tidy(&mut self) {
        let mut seen = std::collections::BTreeSet::new();
        for side in Side::ALL {
            let zone = self.zone_mut(side);
            zone.size = zone.size.filter(|s| s.is_finite()).map(|s| s.clamp(MIN_ZONE, MAX_ZONE));
            for g in &mut zone.groups {
                g.tabs.retain(|p| seen.insert(*p));
                if g.active >= g.tabs.len() {
                    g.active = 0;
                }
                if !g.share.is_finite() || g.share <= 0.0 {
                    g.share = 1.0;
                }
                g.share = g.share.clamp(0.05, 20.0);
            }
            zone.groups.retain(|g| !g.tabs.is_empty());
        }
        self.floating.retain(|f| seen.insert(f.panel));
        for f in &mut self.floating {
            f.at = f.at.filter(|a| a.iter().all(|v| v.is_finite() && v.abs() < 1e6));
            for v in &mut f.size {
                *v = if v.is_finite() { v.clamp(MIN_FLOAT, MAX_ZONE) } else { MIN_FLOAT };
            }
        }
    }
}

/// Where a panel opens when it has never been open: as in Essentials.
fn default_home(p: Panel) -> Home {
    let (side, near) = match p {
        Panel::Properties => (Side::Right, Some(Panel::Align)),
        Panel::Align | Panel::Colour => (Side::Right, Some(Panel::Align)),
        Panel::Library | Panel::MovieExplorer => (Side::Right, Some(Panel::Library)),
        Panel::History => (Side::Right, None),
        Panel::Timeline | Panel::MotionEditor | Panel::Output => (Side::Bottom, Some(Panel::Timeline)),
        Panel::Actions => (Side::Right, None),
    };
    Home { side, near, floating: p == Panel::Actions }
}

// ----- workspaces ---------------------------------------------------------------------------------

/// The built-in workspaces: `(key, name)`. The name is English, translated where shown.
pub const BUILT_IN: [(&str, &str); 3] = [("essentials", tr_noop!("Essentials")), ("animator", tr_noop!("Animator")), ("designer", tr_noop!("Designer"))];

/// Most workspaces of your own kept, and the longest name one has.
pub const MAX_SAVED: usize = 50;
pub const MAX_NAME: usize = 64;

/// A built-in workspace's layout.
pub fn built_in(key: &str) -> Option<Layout> {
    let zone = |groups: Vec<Group>, size: Option<f32>| Zone { groups, size };
    let mut l = match key {
        // Today's layout: Properties, Align, Library with the Movie Explorer, History on the
        // right; the timeline and the Motion Editor below the stage.
        "essentials" => Layout {
            right: zone(vec![Group::of(&[Panel::Properties], 6.4), Group::of(&[Panel::Align], 1.3), Group::of(&[Panel::Library, Panel::MovieExplorer], 3.2), Group::of(&[Panel::History], 1.4)], None),
            bottom: zone(vec![Group::of(&[Panel::Timeline, Panel::MotionEditor], 1.0)], None),
            ..Layout::default()
        },
        // Animation first: a tall timeline with the Output beside it, a slim right dock.
        "animator" => Layout {
            right: zone(vec![Group::of(&[Panel::Properties, Panel::Align], 5.0), Group::of(&[Panel::Library, Panel::MovieExplorer, Panel::History], 4.0)], Some(260.0)),
            bottom: zone(vec![Group::of(&[Panel::Timeline, Panel::MotionEditor], 3.0), Group::of(&[Panel::Output], 1.0)], Some(440.0)),
            ..Layout::default()
        },
        // Drawing first: a wide Properties with Colour and Align, a short timeline.
        "designer" => Layout {
            right: zone(vec![Group::of(&[Panel::Properties], 6.0), Group::of(&[Panel::Colour, Panel::Align], 2.4), Group::of(&[Panel::Library, Panel::MovieExplorer, Panel::History], 3.6)], Some(400.0)),
            bottom: zone(vec![Group::of(&[Panel::Timeline, Panel::MotionEditor], 1.0)], Some(200.0)),
            ..Layout::default()
        },
        _ => return None,
    };
    for p in Panel::ALL {
        if !l.is_open(p) {
            l.homes.insert(p, default_home(p));
        }
    }
    Some(l)
}

/// The workspaces, in the preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspaces {
    /// The one in use: a built-in key or one of `saved`.
    pub current: String,
    /// Each workspace as last arranged (changes are kept until Reset, as in Animate).
    #[serde(deserialize_with = "lenient_map")]
    pub live: BTreeMap<String, Layout>,
    /// Your own, as saved.
    #[serde(deserialize_with = "lenient_map")]
    pub saved: BTreeMap<String, Layout>,
}

impl Default for Workspaces {
    fn default() -> Workspaces {
        Workspaces { current: "essentials".into(), live: BTreeMap::new(), saved: BTreeMap::new() }
    }
}

impl Workspaces {
    /// A workspace as built or saved (what Reset goes back to).
    pub fn base(&self, key: &str) -> Option<Layout> {
        built_in(key).or_else(|| self.saved.get(key).cloned())
    }

    pub fn exists(&self, key: &str) -> bool {
        built_in(key).is_some() || self.saved.contains_key(key)
    }

    /// The layout in use (Essentials when the current one is gone).
    pub fn layout(&self) -> Layout {
        let mut l = self.live.get(&self.current).cloned().or_else(|| self.base(&self.current)).or_else(|| built_in("essentials")).unwrap_or_default();
        l.tidy();
        l
    }

    /// The name shown for a workspace (English for the built-in ones: translate it).
    pub fn name(key: &str) -> &str {
        BUILT_IN.iter().find(|(k, _)| *k == key).map_or(key, |(_, n)| n)
    }

    /// Every workspace's key: the built-in ones, then yours by name.
    pub fn keys(&self) -> Vec<String> {
        BUILT_IN.iter().map(|(k, _)| (*k).to_string()).chain(self.saved.keys().cloned()).collect()
    }

    /// Keeps `layout` as the current workspace's arrangement.
    pub fn keep(&mut self, layout: &Layout) {
        self.live.insert(self.current.clone(), layout.clone());
    }

    /// Switches to `key`, returning its layout.
    pub fn switch(&mut self, key: &str) -> Option<Layout> {
        if !self.exists(key) {
            return None;
        }
        self.current = key.to_string();
        Some(self.layout())
    }

    /// Back to the current workspace as built or saved.
    pub fn reset(&mut self) -> Layout {
        self.live.remove(&self.current);
        self.layout()
    }

    /// Saves `layout` as a workspace of your own called `name` (replacing one of that name),
    /// and makes it the current one. Returns its key.
    pub fn save_as(&mut self, name: &str, layout: &Layout) -> Result<String, &'static str> {
        let name = name.trim();
        if name.is_empty() {
            return Err(tr_noop!("Give the workspace a name"));
        }
        if name.chars().count() > MAX_NAME {
            return Err(tr_noop!("That name is too long"));
        }
        if BUILT_IN.iter().any(|(k, n)| k.eq_ignore_ascii_case(name) || n.eq_ignore_ascii_case(name)) {
            return Err(tr_noop!("That name belongs to a built-in workspace"));
        }
        if !self.saved.contains_key(name) && self.saved.len() >= MAX_SAVED {
            return Err(tr_noop!("There are too many workspaces; replace one of yours"));
        }
        self.saved.insert(name.to_string(), layout.clone());
        self.live.insert(name.to_string(), layout.clone());
        self.current = name.to_string();
        Ok(name.to_string())
    }

    /// Holds what was read from the preferences file to sane bounds.
    pub fn tidy(&mut self) {
        let mut saved: BTreeMap<String, Layout> = std::mem::take(&mut self.saved).into_iter().filter(|(k, _)| !k.trim().is_empty() && k.chars().count() <= MAX_NAME && built_in(k).is_none()).take(MAX_SAVED).collect();
        for l in saved.values_mut() {
            l.tidy();
        }
        self.saved = saved;
        let keys: Vec<String> = self.keys();
        self.live.retain(|k, _| keys.contains(k));
        for l in self.live.values_mut() {
            l.tidy();
        }
        if !self.exists(&self.current) {
            self.current = "essentials".into();
        }
    }
}

// ----- reading older and newer preference files ---------------------------------------------------

/// A list whose items this build can't read (a panel from a newer version) are skipped.
fn lenient_vec<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(raw.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

/// A map whose entries this build can't read are skipped.
fn lenient_map<'de, D: Deserializer<'de>, K: DeserializeOwned + Ord, V: DeserializeOwned>(d: D) -> Result<BTreeMap<K, V>, D::Error> {
    let raw = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
    Ok(raw.into_iter().filter_map(|(k, v)| Some((serde_json::from_value(serde_json::Value::String(k)).ok()?, serde_json::from_value(v).ok()?))).collect())
}

fn lenient_option<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let raw = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(raw).ok())
}

/// The preferences' `workspaces`: anything unreadable gives the defaults rather than losing the
/// rest of the preferences.
pub fn lenient_workspaces<'de, D: Deserializer<'de>>(d: D) -> Result<Workspaces, D::Error> {
    let raw = serde_json::Value::deserialize(d)?;
    let mut w: Workspaces = serde_json::from_value(raw).unwrap_or_default();
    w.tidy();
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn essentials() -> Layout {
        built_in("essentials").expect("built in")
    }

    #[test]
    fn essentials_is_todays_layout() {
        let l = essentials();
        let tabs = |z: &Zone| z.groups.iter().map(|g| g.tabs.clone()).collect::<Vec<_>>();
        assert_eq!(tabs(&l.right), vec![vec![Panel::Properties], vec![Panel::Align], vec![Panel::Library, Panel::MovieExplorer], vec![Panel::History]]);
        assert_eq!(tabs(&l.bottom), vec![vec![Panel::Timeline, Panel::MotionEditor]]);
        assert!(l.left.groups.is_empty() && l.top.groups.is_empty() && l.floating.is_empty());
        assert_eq!((l.right.size, l.bottom.size), (None, None), "sized from the window, as before");
        for p in [Panel::Properties, Panel::Align, Panel::Library, Panel::History, Panel::Timeline] {
            assert!(l.is_shown(p), "{p:?}");
        }
        assert!(!l.is_shown(Panel::MovieExplorer) && l.is_open(Panel::MovieExplorer));
    }

    #[test]
    fn every_panel_is_reachable_in_every_workspace() {
        for (key, _) in BUILT_IN {
            let mut l = built_in(key).expect("built in");
            for p in Panel::ALL {
                l.show(p);
                assert!(l.is_shown(p), "{key}: {p:?} can be brought forward");
                l.close(p);
                assert!(!l.is_open(p), "{key}: {p:?} closes");
                l.toggle(p);
                assert!(l.is_shown(p), "{key}: {p:?} reopens from the Window menu");
            }
            let mut seen = l.order();
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), l.order().len(), "{key}: each panel once");
        }
    }

    #[test]
    fn a_closed_panel_reopens_where_it_was() {
        let mut l = essentials();
        l.close(Panel::Library);
        assert_eq!(l.right.groups[2].tabs, vec![Panel::MovieExplorer]);
        l.show(Panel::Library);
        assert_eq!(l.right.groups[2].tabs, vec![Panel::MovieExplorer, Panel::Library], "back in its group");
        // The last panel of a group: the group goes, and comes back before its neighbour.
        l.close(Panel::Align);
        assert_eq!(l.right.groups.len(), 3);
        l.show(Panel::Align);
        // Its neighbour's group takes it in (as a tab) — it is still on the right.
        assert!(matches!(l.place(Panel::Align), Some(Place::Docked { side: Side::Right, .. })));
        // Never opened: where Essentials keeps it.
        l.show(Panel::Output);
        assert!(matches!(l.place(Panel::Output), Some(Place::Docked { side: Side::Bottom, group: 0, .. })));
        l.show(Panel::Actions);
        assert!(l.is_floating(Panel::Actions), "Actions floats, as it always has");
    }

    #[test]
    fn panels_move_stack_split_and_float() {
        let mut l = essentials();
        // The timeline above the stage (#249).
        l.move_to(Panel::Timeline, Target::Split { side: Side::Top, before: None });
        assert_eq!(l.top.groups.len(), 1);
        assert_eq!(l.bottom.groups[0].tabs, vec![Panel::MotionEditor]);
        // Stacked with the Library as a tab, and shown.
        l.move_to(Panel::History, Target::Stack(Panel::Library));
        assert_eq!(l.right.groups.len(), 3);
        assert!(l.is_shown(Panel::History));
        // Its own group on the left, before nothing.
        l.move_to(Panel::Align, Target::Split { side: Side::Left, before: None });
        assert_eq!(l.left.groups.len(), 1);
        // Before a group.
        l.move_to(Panel::Colour, Target::Split { side: Side::Right, before: Some(Panel::Properties) });
        assert_eq!(l.right.groups[0].tabs, vec![Panel::Colour]);
        // Floated, then docked back where it was.
        l.move_to(Panel::Properties, Target::Float(Some([100.0, 80.0])));
        assert!(l.is_floating(Panel::Properties));
        assert_eq!(l.floating[0].at, Some([100.0, 80.0]));
        l.dock_back(Panel::Properties);
        assert!(matches!(l.place(Panel::Properties), Some(Place::Docked { side: Side::Right, .. })));
        // Onto itself: nothing changes.
        let before = l.clone();
        l.move_to(Panel::Properties, Target::Stack(Panel::Properties));
        assert_eq!(l, before);
        // A floating panel closed reopens floating.
        l.move_to(Panel::Output, Target::Float(None));
        l.close(Panel::Output);
        l.show(Panel::Output);
        assert!(l.is_floating(Panel::Output));
    }

    #[test]
    fn the_layout_round_trips_through_the_preferences() {
        let mut l = built_in("animator").expect("built in");
        l.move_to(Panel::Actions, Target::Float(Some([10.0, 20.0])));
        l.right.size = Some(333.0);
        let mut w = Workspaces::default();
        w.keep(&l);
        let key = w.save_as("Mine", &l).expect("saved");
        let json = serde_json::to_string(&w).expect("json");
        let back: Workspaces = serde_json::from_str(&json).expect("read");
        assert_eq!(back, w);
        assert_eq!(back.current, key);
        assert_eq!(back.layout(), l);
    }

    #[test]
    fn files_from_other_versions_still_open() {
        // A panel from a newer version, a missing field, junk sizes, a repeated panel.
        let json = r#"{"current":"gone","live":{"essentials":{"right":{"groups":[{"tabs":["properties","future_panel","properties"],"active":9,"share":-1}],"size":1e30},"floating":[{"panel":"nope","size":[1,1]},{"panel":"align","size":[1,"x"]},{"panel":"output","size":[10,1e9]}]}},"saved":{"":{}}}"#;
        let w = lenient_workspaces(&mut serde_json::Deserializer::from_str(json)).expect("never fails");
        assert_eq!(w.current, "essentials", "a workspace that's gone: Essentials");
        assert!(w.saved.is_empty());
        let l = w.layout();
        assert_eq!(l.right.groups[0].tabs, vec![Panel::Properties]);
        assert_eq!(l.right.groups[0].active, 0);
        assert_eq!(l.right.size, Some(MAX_ZONE));
        assert_eq!(l.floating.len(), 1);
        assert_eq!(l.floating[0].size, [MIN_FLOAT, MAX_ZONE]);
        // Not even an object: the defaults.
        let w = lenient_workspaces(&mut serde_json::Deserializer::from_str("42")).expect("never fails");
        assert_eq!(w, Workspaces::default());
    }

    #[test]
    fn workspaces_keep_changes_until_reset() {
        let mut w = Workspaces::default();
        let mut l = w.layout();
        assert_eq!(l, essentials());
        l.close(Panel::History);
        w.keep(&l);
        let animator = w.switch("animator").expect("built in");
        assert_eq!(animator, built_in("animator").expect("built in"));
        assert!(!w.switch("essentials").expect("built in").is_open(Panel::History), "kept");
        assert_eq!(w.reset(), essentials(), "Reset brings Essentials back");
        // Yours: saved, changed, reset to as saved.
        let mine = w.layout();
        w.save_as("  Inking ", &mine).expect("saved");
        assert_eq!(w.current, "Inking");
        let mut changed = mine.clone();
        changed.close(Panel::Timeline);
        w.keep(&changed);
        assert_eq!(w.reset(), mine);
        assert!(w.save_as("essentials", &mine).is_err() && w.save_as("Animator", &mine).is_err() && w.save_as(" ", &mine).is_err());
        assert!(w.switch("nope").is_none());
        assert_eq!(w.keys(), vec!["essentials", "animator", "designer", "Inking"]);
    }
}
