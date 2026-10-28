//! A plugin's panel (`panel` in its `plugin.json`): a right pane of texts, checkboxes, sliders,
//! buttons and trees bound to the plugin's commands.
//!
//! The session keeps the panel's state ([`rustgis_engine::plugins::PanelState`]); this draws it
//! and turns the user's changes into commands: a widget with a command runs
//! `plugin:<plugin>/<command>` with `{"panel": {"<widget>": value}}`, one without sets the value
//! with `plugin.panel`. A slider runs its command when it is let go (or stepped, or moved with
//! the keys), never while it is dragged: until then the pane shows the value it would take.

use egui::{Align2, CornerRadius, RichText, Sense, Stroke, Ui, pos2, vec2};
use std::collections::{HashMap, HashSet};

use rustgis_engine::plugins::{Manifest, PanelItemDecl, PanelState, TreeNode, command_id};
use serde_json::{Value, json};

use crate::theme::{Tokens, regular, semibold};
use crate::{RightPane, RustGisApp, icons};

/// The pane's state in the front end.
#[derive(Default)]
pub struct State {
    /// The plugin whose panel the pane shows.
    pub plugin: Option<String>,
    /// A slider being dragged: plugin, widget and the value it shows (not yet sent).
    dragging: Option<(String, String, i64)>,
    /// Each tree's open nodes, and what its search box holds, by widget id.
    open: HashMap<String, HashSet<String>>,
    search: HashMap<String, String>,
}

/// The panel's title (its tab), in the interface language.
pub fn title(app: &RustGisApp) -> String {
    let lang = app.lang().code();
    app.ui
        .plugin_panel
        .plugin
        .as_deref()
        .and_then(rustgis_engine::plugins::get)
        .and_then(|p| p.manifest.panel.as_ref().map(|d| p.manifest.translate(lang, &d.title).to_string()))
        .unwrap_or_else(|| tl!("Plugin").to_string())
}

/// `ui.pane.plugin`: `{"id": plugin}` opens that plugin's panel (running its state command) or,
/// when it is in view, closes it; with no id, shows or hides the panel last opened.
pub fn open(app: &mut RustGisApp, p: &Value) -> Result<Value, String> {
    let Some(id) = p.get("id").and_then(Value::as_str) else {
        if app.ui.plugin_panel.plugin.is_none() {
            return Err("name the plugin whose panel to open: {\"id\": plugin id}".into());
        }
        app.ui.toggle_right(RightPane::Plugin);
        return Ok(Value::Null);
    };
    if app.ui.right == Some(RightPane::Plugin) && app.ui.plugin_panel.plugin.as_deref() == Some(id) {
        app.ui.close_right(RightPane::Plugin);
        return Ok(Value::Null);
    }
    let plugin = rustgis_engine::plugins::get(id).ok_or_else(|| format!("no plugin '{id}'"))?;
    if !plugin.enabled {
        return Err(format!("the plugin {} is disabled", plugin.manifest.name));
    }
    if plugin.manifest.panel.is_none() {
        return Err(format!("{} has no panel", plugin.manifest.name));
    }
    app.ui.plugin_panel = State { plugin: Some(id.to_string()), ..State::default() };
    app.ui.open_right(RightPane::Plugin);
    // The panel opens even if its state command fails: the error is in the status bar.
    if let Err(e) = app.run("plugin.panel", json!({"id": id, "refresh": true})) {
        app.session.notify(e, true);
    }
    Ok(Value::Null)
}

fn run(app: &mut RustGisApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.session.notify(e, true);
    }
}

/// Sends widget `id`'s new value: through its command, or kept for the plugin's next one.
fn send(app: &mut RustGisApp, plugin: &str, item: &PanelItemDecl, value: Value) {
    let values = json!({ item.key(): value });
    if item.command.is_empty() {
        run(app, "plugin.panel", json!({"id": plugin, "values": values}));
    } else {
        run(app, &command_id(plugin, &item.command), json!({ "panel": values }));
    }
}

pub fn show(app: &mut RustGisApp, ui: &mut Ui, t: &Tokens) {
    let Some(pid) = app.ui.plugin_panel.plugin.clone() else {
        ui.label(RichText::new(tl!("No plugin panel is open.")).color(t.text_dim));
        return;
    };
    // A plugin turned off or removed takes its panel with it.
    let plugin = rustgis_engine::plugins::get(&pid).filter(|p| p.enabled);
    let (Some(plugin), Some(state)) = (plugin, app.session.plugin_panel(&pid).cloned()) else {
        app.ui.plugin_panel = State::default();
        app.ui.close_right(RightPane::Plugin);
        return;
    };
    let m = &plugin.manifest;
    let Some(decl) = m.panel.as_ref() else { return };
    let lang = app.lang().code();
    let tr = |s: &str| m.translate(lang, s).to_string();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        let icon = if icons::ALL.contains(&decl.icon.as_str()) { decl.icon.as_str() } else { "plugin" };
        icons::paint(ui.painter(), r, icon, t.icon, t.accent);
        ui.label(RichText::new(tr(&decl.title)).font(semibold(15.0)));
    });
    if !decl.help.is_empty() {
        ui.label(RichText::new(tr(&decl.help)).size(11.5).color(t.text_dim));
    }
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
    let items: Vec<&PanelItemDecl> = decl.items.iter().filter(|d| state.items.get(d.key()).is_none_or(|s| s.visible)).collect();
    let mut i = 0;
    while let Some(item) = items.get(i).copied() {
        // Buttons side by side, wrapping, as many as follow each other.
        if item.kind == "button" {
            let n = items.iter().skip(i).take_while(|d| d.kind == "button").count();
            ui.horizontal_wrapped(|ui| {
                for b in items.iter().skip(i).take(n) {
                    button(app, ui, &pid, m, b, &state, &tr);
                }
            });
            ui.add_space(8.0);
            i += n;
            continue;
        }
        match item.kind.as_str() {
            "text" => text(ui, item, &state, &tr, t),
            "checkbox" => checkbox(app, ui, &pid, item, &state, &tr),
            "slider" => slider(app, ui, &pid, item, &state, &tr, t),
            "tree" => tree(app, ui, &pid, item, &state, &tr, t),
            _ => {}
        }
        ui.add_space(8.0);
        i += 1;
    }
}

/// A widget's label: what the plugin set, or the declared one (translated).
fn label_of(item: &PanelItemDecl, state: &PanelState, tr: &dyn Fn(&str) -> String) -> String {
    state.items.get(item.key()).and_then(|s| s.label.clone()).unwrap_or_else(|| tr(&item.label))
}

fn enabled(item: &PanelItemDecl, state: &PanelState) -> bool {
    state.items.get(item.key()).is_none_or(|s| s.enabled)
}

fn text(ui: &mut Ui, item: &PanelItemDecl, state: &PanelState, tr: &dyn Fn(&str) -> String, t: &Tokens) {
    let label = label_of(item, state, tr);
    if !label.is_empty() {
        ui.label(RichText::new(label).size(11.0).color(t.text_dim));
    }
    let body = state.items.get(item.key()).map(|s| s.text.as_str()).unwrap_or_default();
    let color = if enabled(item, state) { t.text } else { t.text_disabled };
    let resp = ui.add(egui::Label::new(RichText::new(if body.is_empty() { "—" } else { body }).size(12.5).color(color)).wrap());
    if !item.help.is_empty() {
        resp.on_hover_text(tr(&item.help));
    }
}

fn checkbox(app: &mut RustGisApp, ui: &mut Ui, pid: &str, item: &PanelItemDecl, state: &PanelState, tr: &dyn Fn(&str) -> String) {
    let mut on = state.checked(item.key());
    let resp = ui.add_enabled(enabled(item, state), egui::Checkbox::new(&mut on, label_of(item, state, tr)));
    let resp = if item.help.is_empty() { resp } else { resp.on_hover_text(tr(&item.help)) };
    if resp.changed() {
        send(app, pid, item, json!(on));
    }
}

fn button(
    app: &mut RustGisApp,
    ui: &mut Ui,
    pid: &str,
    m: &Manifest,
    item: &PanelItemDecl,
    state: &PanelState,
    tr: &dyn Fn(&str) -> String,
) {
    let cmd = m.commands.iter().find(|c| c.id == item.command);
    let label = state
        .items
        .get(item.key())
        .and_then(|s| s.label.clone())
        .or_else(|| (!item.label.is_empty()).then(|| tr(&item.label)))
        .or_else(|| cmd.as_ref().map(|c| tr(&c.label)))
        .unwrap_or_default()
        .replace('\n', " ");
    let help = if item.help.is_empty() { cmd.as_ref().map(|c| tr(&c.help)).unwrap_or_default() } else { tr(&item.help) };
    let resp = ui.add_enabled(enabled(item, state), egui::Button::new(label).min_size(vec2(64.0, 26.0)));
    let resp = if help.is_empty() { resp } else { resp.on_hover_text(help) };
    if resp.clicked() {
        run(app, &command_id(pid, &item.command), Value::Null);
    }
}

/// A 26 px button with a chevron pointing left or right, found by `tip`.
fn step_button(ui: &mut Ui, on: bool, left: bool, tip: &str, t: &Tokens) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), if on { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, tip));
    if on && resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
    }
    ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let (c, d) = (r.center(), if left { 2.5 } else { -2.5 });
    let ink = if on { t.text } else { t.text_disabled };
    let stroke = Stroke::new(1.6, ink);
    ui.painter().line_segment([pos2(c.x + d, c.y - 5.0), pos2(c.x - d, c.y)], stroke);
    ui.painter().line_segment([pos2(c.x - d, c.y), pos2(c.x + d, c.y + 5.0)], stroke);
    resp.on_hover_text(tip)
}

fn slider(app: &mut RustGisApp, ui: &mut Ui, pid: &str, item: &PanelItemDecl, state: &PanelState, tr: &dyn Fn(&str) -> String, t: &Tokens) {
    let id = item.key();
    let Some(st) = state.items.get(id) else { return };
    let (min, max) = (st.min, st.max.max(st.min));
    let stored = state.slider(id);
    let dragging = app.ui.plugin_panel.dragging.as_ref().filter(|(p, i, _)| p == pid && i == id).map(|d| d.2);
    let mut v = dragging.unwrap_or(stored).clamp(min, max);
    let on = enabled(item, state) && max > min;
    // The caption, and the value it would take (shown as it is dragged).
    ui.horizontal(|ui| {
        let label = label_of(item, state, tr);
        ui.label(RichText::new(label).size(11.0).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let color = if dragging.is_some() { t.accent_text } else { t.text };
            ui.label(RichText::new(state.slider_label(id, v)).font(semibold(12.5)).color(color));
        });
    });
    let mut stepped = None;
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let buttons = if item.steps { 2.0 * (26.0 + gap) } else { 0.0 };
        let room = ui.available_width();
        if item.steps && step_button(ui, on && v > min, true, tl!("Step back"), t).clicked() {
            stepped = Some(v - 1);
        }
        ui.spacing_mut().slider_width = (room - buttons - 4.0).max(60.0);
        let resp = ui.add_enabled(on, egui::Slider::new(&mut v, min..=max).show_value(false));
        let resp = if item.help.is_empty() { resp } else { resp.on_hover_text(tr(&item.help)) };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, on, label_of(item, state, tr)));
        if item.steps && step_button(ui, on && v < max, false, tl!("Step forward"), t).clicked() {
            stepped = Some(v + 1);
        }
        if resp.dragged() {
            app.ui.plugin_panel.dragging = Some((pid.to_string(), id.to_string(), v));
        } else if dragging.is_some() || v != stored {
            // Let go, clicked or moved with the keys: now it runs. A step button clicked on the
            // same frame wins (its value was taken from the slider's before this frame's move).
            // A range the plugin changed during a drag clamps the value each frame, and on release.
            app.ui.plugin_panel.dragging = None;
            if v != stored {
                stepped.get_or_insert(v);
            }
        }
    });
    // Its ends, and where it is among them.
    ui.horizontal(|ui| {
        let small = |s: String| RichText::new(s).size(10.5).color(t.text_dim);
        ui.label(small(state.slider_label(id, min)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(small(state.slider_label(id, max)));
            let of = crate::i18n::fmt("{n} of {count}", &[("n", &(v - min + 1).to_string()), ("count", &(max - min + 1).to_string())]);
            let r = ui.available_rect_before_wrap();
            ui.painter().text(pos2(r.center().x, r.center().y), Align2::CENTER_CENTER, of, regular(10.5), t.text_dim);
        });
    });
    if let Some(n) = stepped.map(|n| n.clamp(min, max)).filter(|n| *n != stored) {
        send(app, pid, item, json!(n));
    }
}

/// A tree's row height, and how far each level is indented.
const TREE_ROW: f32 = 22.0;
const TREE_INDENT: f32 = 14.0;

/// A row of a tree as shown: a node, how deep it is, whether its children show below it, and
/// whether they must (a branch above what a search found).
struct TreeRow<'a> {
    node: &'a TreeNode,
    depth: usize,
    open: bool,
    held: bool,
}

/// Lower case without accents, so that "arbol" finds "Árboles".
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// The rows of `nodes`: with no `words`, each node and, when open, its children; searching, the
/// nodes whose label holds every word (shown as they are) and the branches above them, open.
fn tree_rows<'a>(nodes: &'a [TreeNode], depth: usize, words: &[String], open: &HashSet<String>, out: &mut Vec<TreeRow<'a>>) {
    for n in nodes {
        let at = out.len();
        out.push(TreeRow { node: n, depth, open: false, held: false });
        if words.is_empty() || {
            let label = fold(&n.label);
            words.iter().all(|w| label.contains(w.as_str()))
        } {
            if open.contains(&n.id) && !n.children.is_empty() {
                if let Some(r) = out.get_mut(at) {
                    r.open = true;
                }
                tree_rows(&n.children, depth + 1, &[], open, out);
            }
        } else {
            // Kept only for what matches below it.
            tree_rows(&n.children, depth + 1, words, open, out);
            if out.len() == at + 1 {
                out.pop();
            } else if let Some(r) = out.get_mut(at) {
                (r.open, r.held) = (true, true);
            }
        }
    }
}

/// A tree: a row per node, a branch's children indented under it while it is open. A click
/// chooses a node (running the widget's command); the triangle, or a double click, opens or
/// closes a branch (a click on the triangle never chooses it; the first click of a double click
/// does, as a file browser's). With `search`, a box above it keeps the nodes whose label holds the words
/// typed, with the branches above them held open (a branch found opens and closes as ever).
fn tree(app: &mut RustGisApp, ui: &mut Ui, pid: &str, item: &PanelItemDecl, state: &PanelState, tr: &dyn Fn(&str) -> String, t: &Tokens) {
    let id = item.key();
    let Some(st) = state.items.get(id) else { return };
    let label = label_of(item, state, tr);
    if !label.is_empty() {
        ui.label(RichText::new(label).size(11.0).color(t.text_dim));
    }
    let on = enabled(item, state);
    if item.search {
        // As the Layers pane's filter box: the shared search field, over a separator.
        let query = app.ui.plugin_panel.search.entry(id.to_string()).or_default();
        ui.add_enabled_ui(on, |ui| crate::widgets::search_field(ui, query, tl!("Search")));
        ui.separator();
    }
    let words: Vec<String> =
        app.ui.plugin_panel.search.get(id).map(|q| fold(q).split_whitespace().map(String::from).collect()).unwrap_or_default();
    let none = HashSet::new();
    let open = app.ui.plugin_panel.open.get(id).unwrap_or(&none);
    let mut rows = Vec::new();
    tree_rows(&st.nodes, 0, &words, open, &mut rows);
    if rows.is_empty() {
        let empty = if words.is_empty() { "—" } else { tl!("Nothing matches.") };
        ui.label(RichText::new(empty).size(12.5).color(t.text_dim));
        return;
    }
    let chosen = state.chosen(id);
    let (mut toggle, mut pick) = (None, None);
    egui::ScrollArea::vertical().id_salt(("plugin-tree", pid, id)).max_height(320.0).auto_shrink([false, true]).show_rows(
        ui,
        TREE_ROW,
        rows.len(),
        |ui, range| {
            for r in rows.iter().skip(range.start).take(range.len()) {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), TREE_ROW), if on { Sense::click() } else { Sense::hover() });
                let here = chosen == Some(r.node.id.as_str());
                if here {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), t.checked);
                } else if on && resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
                }
                let ink = if on { t.text } else { t.text_disabled };
                let x = rect.left() + 4.0 + r.depth as f32 * TREE_INDENT;
                let branch = !r.node.children.is_empty();
                let arrow = egui::Rect::from_min_size(pos2(x, rect.top()), vec2(TREE_INDENT, TREE_ROW));
                if branch {
                    let c = arrow.center();
                    let pts = if r.open {
                        vec![pos2(c.x - 4.0, c.y - 2.0), pos2(c.x + 4.0, c.y - 2.0), pos2(c.x, c.y + 3.0)]
                    } else {
                        vec![pos2(c.x - 2.0, c.y - 4.0), pos2(c.x + 3.0, c.y), pos2(c.x - 2.0, c.y + 4.0)]
                    };
                    ui.painter().add(egui::Shape::convex_polygon(pts, t.text_dim, Stroke::NONE));
                }
                // The count at the right, the label in what's left of the row.
                let mut right = rect.right() - 6.0;
                if let Some(n) = r.node.count {
                    let g = ui.painter().layout_no_wrap(format!("({n})"), regular(12.0), t.text_dim);
                    right -= g.size().x;
                    ui.painter().galley(pos2(right, rect.center().y - g.size().y / 2.0), g, t.text_dim);
                    right -= 8.0;
                }
                let text = egui::Rect::from_min_max(pos2(arrow.right() + 2.0, rect.top()), pos2(right, rect.bottom()));
                let font = if branch { semibold(12.5) } else { regular(12.5) };
                ui.painter().with_clip_rect(text).text(pos2(text.left(), rect.center().y), Align2::LEFT_CENTER, &r.node.label, font, ink);
                let resp = resp.on_hover_text(r.node.label.as_str());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, on, here, &r.node.label));
                // A branch held open over what a search found doesn't close.
                let on_arrow = resp.interact_pointer_pos().is_some_and(|p| arrow.contains(p));
                if branch && !r.held && (resp.double_clicked() || (resp.clicked() && on_arrow)) {
                    toggle = Some(r.node.id.clone());
                } else if resp.clicked() && !here && !(branch && on_arrow) {
                    pick = Some(r.node.id.clone());
                }
            }
        },
    );
    if let Some(n) = toggle {
        let open = app.ui.plugin_panel.open.entry(id.to_string()).or_default();
        if !open.remove(&n) {
            open.insert(n);
        }
    }
    if let Some(n) = pick {
        send(app, pid, item, json!(n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, label: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode { id: id.into(), label: label.into(), count: None, children }
    }

    /// The rows shown for `query` with `open` branches: each id indented by its depth, `/` when
    /// its children show, `*` when a search holds it open.
    fn shown(nodes: &[TreeNode], query: &str, open: &[&str]) -> Vec<String> {
        let words: Vec<String> = fold(query).split_whitespace().map(String::from).collect();
        let open: HashSet<String> = open.iter().map(|s| s.to_string()).collect();
        let mut rows = Vec::new();
        tree_rows(nodes, 0, &words, &open, &mut rows);
        rows.iter()
            .map(|r| format!("{}{}{}{}", "  ".repeat(r.depth), r.node.id, if r.open { "/" } else { "" }, if r.held { "*" } else { "" }))
            .collect()
    }

    /// A tree shows its open branches' children; a search keeps what matches every word (case and
    /// accents aside) with the branches above it held open, and a branch found opens as ever.
    #[test]
    fn tree_rows_follow_open_branches_and_searches() {
        let tree = vec![
            node(
                "env",
                "Ambiente",
                vec![node("trees", "Arborización", vec![node("601", "Árboles urbanos", vec![])]), node("water", "Agua", vec![])],
            ),
            node("cul", "Cultura", vec![node("c1", "Teatro", vec![])]),
        ];
        assert_eq!(shown(&tree, "", &[]), ["env", "cul"]);
        assert_eq!(shown(&tree, "", &["env"]), ["env/", "  trees", "  water", "cul"]);
        assert_eq!(shown(&tree, "", &["trees"]), ["env", "cul"], "an open branch under a closed one stays hidden");
        assert_eq!(shown(&tree, "arbol", &[]), ["env/*", "  trees/*", "    601"]);
        assert_eq!(shown(&tree, "ÁRBOLES urb", &[]), ["env/*", "  trees/*", "    601"], "every word, case and accents aside");
        assert_eq!(shown(&tree, "cultura", &[]), ["cul"], "a branch found shows as it is");
        assert_eq!(shown(&tree, "cultura", &["cul"]), ["cul/", "  c1"], "and opens as ever");
        assert!(shown(&tree, "zzz", &["env"]).is_empty());
        assert_eq!(fold("Ñandú Çà"), "nandu ca");
    }
}
