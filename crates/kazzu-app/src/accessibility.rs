use std::collections::HashMap;

use serde::Serialize;
use serde_json::json;
use slint::Model;

use crate::ui::{AppBridge, Tile, Track};

const LIMIT: usize = 16;

#[derive(Serialize)]
struct Node {
    id: String,
    role: &'static str,
    label: String,
    description: String,
    parent: String,
    checked: Option<bool>,
    value: String,
    click: String,
    increment: String,
    decrement: String,
    delete: String,
    children: Vec<String>,
}

impl Node {
    fn new(
        id: impl Into<String>,
        role: &'static str,
        label: impl Into<String>,
        description: impl Into<String>,
        parent: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            role,
            label: label.into(),
            description: description.into(),
            parent: parent.into(),
            checked: None,
            value: String::new(),
            click: String::new(),
            increment: String::new(),
            decrement: String::new(),
            delete: String::new(),
            children: Vec::new(),
        }
    }

    fn text(id: impl Into<String>, label: impl Into<String>, parent: &str) -> Self {
        Self::new(id, "text", label, "", parent)
    }
}

fn group(id: &str, label: &str, parent: &str) -> Node {
    Node::new(id, "group", label, "", parent)
}

fn track_label(track: &Track) -> String {
    let title = if track.title.is_empty() { "Faixa sem título" } else { track.title.as_str() };
    if track.artist.is_empty() {
        title.to_string()
    } else {
        format!("{title}, {}", track.artist.as_str())
    }
}

fn add_tracks(
    nodes: &mut Vec<Node>,
    model: slint::ModelRc<Track>,
    parent: &str,
    prefix: &str,
    mut click: impl FnMut(&Track, usize) -> String,
    mut delete: impl FnMut(&Track, usize) -> String,
) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut children = Vec::new();
    for index in 0..model.row_count().min(LIMIT) {
        let Some(track) = model.row_data(index) else { continue };
        let key = if track.id.is_empty() {
            format!("{prefix}:index-{index}")
        } else {
            let ordinal = seen.entry(track.id.to_string()).or_insert(0);
            let id = format!("{prefix}:{}:{ordinal}", track.id.as_str());
            *ordinal += 1;
            id
        };
        let description = if track.local {
            format!("Música local · {}", track.duration_label.as_str())
        } else {
            format!("Streaming · {}", track.duration_label.as_str())
        };
        let mut node = Node::new(
            key.clone(),
            "button",
            track_label(&track),
            description,
            parent,
        );
        node.value = track.duration_label.to_string();
        node.click = click(&track, index);
        node.delete = delete(&track, index);
        children.push(key);
        nodes.push(node);
    }
    children
}

fn add_tiles(
    nodes: &mut Vec<Node>,
    model: slint::ModelRc<Tile>,
    parent: &str,
    prefix: &str,
    mut click: impl FnMut(&Tile) -> String,
) -> Vec<String> {
    let mut children = Vec::new();
    for index in 0..model.row_count().min(LIMIT) {
        let Some(tile) = model.row_data(index) else { continue };
        let id = if tile.id.is_empty() {
            format!("{prefix}:index-{index}")
        } else {
            format!("{prefix}:{}", tile.id.as_str())
        };
        let mut node = Node::new(id.clone(), "button", tile.title.as_str(), tile.sub.as_str(), parent);
        node.click = click(&tile);
        children.push(id);
        nodes.push(node);
    }
    children
}

fn navigation(nodes: &mut Vec<Node>, root: &mut Node) {
    let mut group = group("navigation", "Navegação", "root");
    let labels = ["Início", "Explorar", "Biblioteca", "Buscar", "Configurações"];
    for (index, label) in labels.into_iter().enumerate() {
        let id = format!("navigation:{index}");
        let mut node = Node::new(id.clone(), "button", label, "", "navigation");
        node.click = format!("a11y:navigate:{index}");
        group.children.push(id);
        nodes.push(node);
    }
    root.children.push("navigation".into());
    nodes.push(group);
}

fn transport(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("transport", "Reprodução", "root");
    let now_title = bridge.get_now().title;
    let state = if bridge.get_playing() { "Reproduzindo" } else { "Pausado" };
    let label = if now_title.is_empty() {
        state.to_string()
    } else {
        format!("{state} {}", now_title.as_str())
    };
    let mut play = Node::new("transport:play", "button", label, "", "transport");
    play.click = "a11y:toggle-play".into();
    play.value = bridge.get_position().as_str().into();
    group.children.push("transport:play".into());
    nodes.push(play);

    for (id, label, action) in [
        ("transport:previous", "Faixa anterior", "a11y:previous"),
        ("transport:next", "Próxima faixa", "a11y:next"),
    ] {
        let mut node = Node::new(id, "button", label, "", "transport");
        node.click = action.into();
        group.children.push(id.into());
        nodes.push(node);
    }

    let mut seek = Node::new("transport:seek", "progressbar", "Posição da faixa", "", "transport");
    seek.value = format!("{} restantes", bridge.get_remaining().as_str());
    seek.increment = "a11y:seek:15".into();
    seek.decrement = "a11y:seek:-15".into();
    group.children.push("transport:seek".into());
    nodes.push(seek);

    let mut volume = Node::new("transport:volume", "slider", "Volume", "", "transport");
    volume.value = format!("{}%", (bridge.get_volume() * 100.0).round() as i32);
    volume.increment = "a11y:volume:1".into();
    volume.decrement = "a11y:volume:-1".into();
    group.children.push("transport:volume".into());
    nodes.push(volume);

    let now = bridge.get_now();
    for (id, label, action, checked) in [
        ("transport:like", "Curtir faixa", format!("a11y:like:{}", now.id.as_str()), bridge.get_liked()),
        ("transport:shuffle", "Reprodução aleatória", "a11y:shuffle".into(), bridge.get_shuffle()),
        ("transport:repeat", "Repetir", "a11y:repeat".into(), bridge.get_repeat()),
    ] {
        let mut node = Node::new(id, "checkbox", label, "", "transport");
        node.checked = Some(checked);
        node.click = action;
        group.children.push(id.into());
        nodes.push(node);
    }

    for (id, label, action) in [
        ("transport:lyrics", "Mostrar letras", "a11y:lyrics"),
        ("transport:share", "Compartilhar faixa", "a11y:share"),
        ("transport:watch", "Assistir videoclipe", "a11y:watch"),
    ] {
        let mut node = Node::new(id, "button", label, "", "transport");
        node.click = action.into();
        group.children.push(id.into());
        nodes.push(node);
    }

    root.children.push("transport".into());
    nodes.push(group);
}

fn queue(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("queue", "Fila de reprodução", "root");
    let ids = bridge.get_queue_ids();
    group.children = add_tracks(
        nodes,
        bridge.get_queue(),
        "queue",
        "queue",
        |_, index| {
            let id = ids.row_data(index).unwrap_or_default();
            format!("a11y:queue:jump:{}", id.as_str())
        },
        |_, index| {
            let id = ids.row_data(index).unwrap_or_default();
            format!("a11y:queue:remove:{}", id.as_str())
        },
    );
    if group.children.is_empty() {
        group.children.push("queue:empty".into());
        nodes.push(Node::new("queue:empty", "text", "A fila está vazia.", "", "queue"));
    }
    root.children.push("queue".into());
    nodes.push(group);
}

fn home(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("home", "Início", "root");
    group.children.extend(add_tracks(
        nodes,
        bridge.get_quick_picks(),
        "home",
        "home:quick",
        |track, _| format!("a11y:play:{}", track.id.as_str()),
        |_, _| String::new(),
    ));
    group.children.extend(add_tiles(
        nodes,
        bridge.get_suggestions(),
        "home",
        "home:suggestion",
        |tile| format!("a11y:open:{}:{}", tile.kind.as_str(), tile.id.as_str()),
    ));
    root.children.push("home".into());
    nodes.push(group);
}

fn explore(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("explore", "Explorar", "root");
    group.children.extend(add_tiles(
        nodes,
        bridge.get_categories(),
        "explore",
        "explore:category",
        |tile| format!("a11y:open:{}:{}", tile.kind.as_str(), tile.id.as_str()),
    ));
    group.children.extend(add_tiles(
        nodes,
        bridge.get_new_releases(),
        "explore",
        "explore:release",
        |tile| format!("a11y:open:{}:{}", tile.kind.as_str(), tile.id.as_str()),
    ));
    group.children.extend(add_tracks(
        nodes,
        bridge.get_trending(),
        "explore",
        "explore:trending",
        |track, _| format!("a11y:play:{}", track.id.as_str()),
        |_, _| String::new(),
    ));
    root.children.push("explore".into());
    nodes.push(group);
}

fn library(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("library", "Biblioteca", "root");
    group.children.extend(add_tiles(
        nodes,
        bridge.get_playlists(),
        "library",
        "library:playlist",
        |tile| format!("a11y:open:{}:{}", tile.kind.as_str(), tile.id.as_str()),
    ));
    group.children.extend(add_tiles(
        nodes,
        bridge.get_library_albums(),
        "library",
        "library:album",
        |tile| format!("a11y:open:{}:{}", tile.kind.as_str(), tile.id.as_str()),
    ));
    group.children.extend(add_tiles(
        nodes,
        bridge.get_artists(),
        "library",
        "library:artist",
        |tile| format!("a11y:open:{}:{}", tile.kind.as_str(), tile.id.as_str()),
    ));
    root.children.push("library".into());
    nodes.push(group);
}

fn search(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("search", "Busca", "root");
    let recents = bridge.get_recent_searches();
    for index in 0..recents.row_count().min(LIMIT) {
        let Some(value) = recents.row_data(index) else { continue };
        if value.is_empty() {
            continue;
        }
        let id = format!("search:recent:{index}");
        let mut node = Node::new(id.clone(), "button", format!("Buscar {value}"), "", "search");
        node.click = format!("a11y:search:{}", value.as_str());
        group.children.push(id);
        nodes.push(node);
    }
    group.children.extend(add_tracks(
        nodes,
        bridge.get_results(),
        "search",
        "search:result",
        |track, _| format!("a11y:play:{}", track.id.as_str()),
        |_, _| String::new(),
    ));
    if group.children.is_empty() {
        group.children.push("search:empty".into());
        nodes.push(Node::new("search:empty", "text", bridge.get_search_message().as_str(), "", "search"));
    }
    root.children.push("search".into());
    nodes.push(group);
}

fn detail(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    if !bridge.get_detail_open() {
        return;
    }
    let mut group = group("detail", bridge.get_detail_title().as_str(), "root");
    for (id, label, action) in [
        ("detail:play", "Reproduzir tudo", "a11y:detail-play"),
        ("detail:close", "Fechar detalhe", "a11y:detail-close"),
    ] {
        let mut node = Node::new(id, "button", label, "", "detail");
        node.click = action.into();
        group.children.push(id.into());
        nodes.push(node);
    }
    group.children.extend(add_tracks(
        nodes,
        bridge.get_detail_tracks(),
        "detail",
        "detail:track",
        |track, _| format!("a11y:play:{}", track.id.as_str()),
        |_, _| String::new(),
    ));
    if !bridge.get_detail_message().is_empty() {
        let id = "detail:message";
        group.children.push(id.into());
        nodes.push(Node::new(id, "text", bridge.get_detail_message().as_str(), "", "detail"));
    }
    root.children.push("detail".into());
    nodes.push(group);
}

fn settings(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("settings", "Configurações", "root");
    let motion = bridge.get_app_visible() && !bridge.get_reduce_motion() && bridge.get_effects() > 0;
    let toggles = [
        ("settings:cover", "Capa como tema", "cover", bridge.get_cover_theme()),
        ("settings:dark", "Tema escuro", "dark", bridge.get_dark()),
        ("settings:rounded", "Formas arredondadas", "rounded", bridge.get_rounded()),
        ("settings:motion", "Animações", "motion", motion),
        ("settings:autoplay", "Reprodução automática", "autoplay", bridge.get_autoplay()),
    ];
    for (id, label, name, checked) in toggles {
        let mut node = Node::new(id, "checkbox", label, "", "settings");
        node.checked = Some(checked);
        node.click = format!("a11y:setting:{name}:{}", i32::from(!checked));
        group.children.push(id.into());
        nodes.push(node);
    }

    for minutes in [0, 15, 30, 60] {
        let id = format!("settings:sleep:{minutes}");
        let mut node = Node::new(id.clone(), "button", if minutes == 0 { "Desligar timer de sono" } else { "Timer de sono" }, "", "settings");
        node.value = if minutes == 0 { "Desligado".into() } else { format!("{minutes} minutos") };
        node.click = format!("a11y:sleep:{minutes}");
        group.children.push(id);
        nodes.push(node);
    }

    let outputs = bridge.get_outputs();
    for index in 0..outputs.row_count().min(LIMIT) {
        let Some(output) = outputs.row_data(index) else { continue };
        let id = format!("settings:output:{index}");
        let mut node = Node::new(id.clone(), "button", output.title.as_str(), "", "settings");
        node.click = format!("a11y:output:{}", output.id.as_str());
        group.children.push(id);
        nodes.push(node);
    }

    root.children.push("settings".into());
    nodes.push(group);
}

fn status(nodes: &mut Vec<Node>, root: &mut Node, bridge: &AppBridge<'_>) {
    let mut group = group("status", "Estado do aplicativo", "root");
    let entries = [
        ("status:notice", bridge.get_notice().as_str().to_string()),
        ("status:room", bridge.get_room_status().as_str().to_string()),
        ("status:lastfm", bridge.get_lastfm_status().as_str().to_string()),
        ("status:search", bridge.get_search_message().as_str().to_string()),
        ("status:storage", format!("Persistentes {}, cache {}", bridge.get_persistent_usage().as_str(), bridge.get_cache_usage().as_str())),
        ("status:version", format!("Kazzu {}", bridge.get_version().as_str())),
    ];
    for (id, label) in entries {
        if label.is_empty() {
            continue;
        }
        group.children.push(id.into());
        nodes.push(Node::text(id, label, "status"));
    }
    root.children.push("status".into());
    nodes.push(group);
}

pub(crate) fn tree(bridge: &AppBridge<'_>) -> String {
    let mut nodes = Vec::new();
    let mut root = Node::new("root", "group", "Kazzu", "Player de música", "");
    navigation(&mut nodes, &mut root);
    transport(&mut nodes, &mut root, bridge);
    queue(&mut nodes, &mut root, bridge);
    home(&mut nodes, &mut root, bridge);
    explore(&mut nodes, &mut root, bridge);
    library(&mut nodes, &mut root, bridge);
    search(&mut nodes, &mut root, bridge);
    detail(&mut nodes, &mut root, bridge);
    settings(&mut nodes, &mut root, bridge);
    status(&mut nodes, &mut root, bridge);
    nodes.push(root);
    json!({ "nodes": nodes }).to_string()
}
