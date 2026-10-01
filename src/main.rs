use adw::prelude::*;
use gtk::{gdk, gio, glib};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};
use std::{collections::VecDeque, time::SystemTime};

const APP_ID: &str = "io.github.membboard.MemBoard";
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];
const RECENT_GROUP: &str = "Последние";
const RECENT_LIMIT: usize = 60;

#[derive(Default, Deserialize, Serialize)]
struct Settings {
    sticker_directory: Option<PathBuf>,
    #[serde(default)]
    recent_stickers: Vec<PathBuf>,
    #[serde(default)]
    close_on_copy: bool,
}

struct Animation {
    picture: glib::WeakRef<gtk::Picture>,
    iterator: gdk_pixbuf::PixbufAnimationIter,
}

struct State {
    directory: Option<PathBuf>,
    groups: BTreeMap<String, Vec<PathBuf>>,
    active: String,
    flow: gtk::FlowBox,
    tabs: gtk::Box,
    stack: gtk::Stack,
    empty: adw::StatusPage,
    search: gtk::SearchEntry,
    count: gtk::Label,
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    clear_recent: gtk::Button,
    recent: Vec<PathBuf>,
    close_on_copy: bool,
    animations: Vec<Animation>,
    render_generation: u64,
    clipboard_texture: Option<gdk::Texture>,
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Mem Board")
        .default_width(760)
        .default_height(560)
        .build();
    window.set_size_request(460, 360);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);
    let choose = gtk::Button::from_icon_name("folder-open-symbolic");
    choose.set_tooltip_text(Some("Выбрать папку со стикерами"));
    header.pack_start(&choose);
    let recent = gtk::Button::with_label("Последние стикеры");
    header.pack_start(&recent);
    let preferences = gtk::Button::from_icon_name("preferences-system-symbolic");
    preferences.set_tooltip_text(Some("Настройки"));
    header.pack_start(&preferences);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Обновить список"));
    header.pack_end(&refresh);
    let clear_recent = gtk::Button::from_icon_name("edit-clear-symbolic");
    clear_recent.set_tooltip_text(Some("Очистить последние стикеры"));
    clear_recent.set_visible(false);
    header.pack_end(&clear_recent);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Поиск по имени стикера")
        .hexpand(true)
        .build();
    header.set_title_widget(Some(&search));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let tab_scroller = gtk::ScrolledWindow::builder()
        // Keep browser-like horizontal overflow, but do not show a scrollbar.
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    tabs.set_margin_start(12);
    tabs.set_margin_end(12);
    tabs.set_margin_top(8);
    tabs.set_margin_bottom(2);
    tab_scroller.set_child(Some(&tabs));
    content.append(&tab_scroller);
    let count = gtk::Label::builder().xalign(0.0).build();
    count.add_css_class("dim-label");
    count.set_margin_start(18);
    count.set_margin_end(18);
    count.set_margin_top(10);
    count.set_margin_bottom(8);
    content.append(&count);
    let stack = gtk::Stack::builder().vexpand(true).build();
    let empty = adw::StatusPage::builder().icon_name("image-x-generic-symbolic").title("Выберите папку со стикерами").description("Поддерживаются PNG, JPEG, WebP, GIF и BMP. Нажмите на стикер, чтобы скопировать его в буфер обмена.").build();
    let select = gtk::Button::with_label("Выбрать папку");
    select.add_css_class("suggested-action");
    empty.set_child(Some(&select));
    stack.add_named(&empty, Some("empty"));
    let scroller = gtk::ScrolledWindow::builder().vexpand(true).build();
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_min_children_per_line(3);
    flow.set_max_children_per_line(8);
    flow.set_row_spacing(12);
    flow.set_column_spacing(12);
    flow.set_margin_top(12);
    flow.set_margin_bottom(18);
    flow.set_margin_start(18);
    flow.set_margin_end(18);
    flow.set_homogeneous(true);
    scroller.set_child(Some(&flow));
    stack.add_named(&scroller, Some("stickers"));
    content.append(&stack);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&content));
    toolbar.set_content(Some(&toasts));
    window.set_content(Some(&toolbar));

    let settings = load_settings();
    let state = Rc::new(RefCell::new(State {
        directory: settings.sticker_directory.filter(|p| p.is_dir()),
        groups: BTreeMap::new(),
        active: "Все".into(),
        flow,
        tabs,
        stack,
        empty,
        search: search.clone(),
        count,
        window: window.clone(),
        toasts,
        clear_recent: clear_recent.clone(),
        recent: settings
            .recent_stickers
            .into_iter()
            .filter(|p| p.is_file())
            .collect(),
        close_on_copy: settings.close_on_copy,
        animations: Vec::new(),
        render_generation: 0,
        clipboard_texture: None,
    }));
    chooser(&choose, &state);
    chooser(&select, &state);
    {
        let state = state.clone();
        recent.connect_clicked(move |_| {
            state.borrow_mut().active = RECENT_GROUP.into();
            render(&state);
        });
    }
    {
        let state = state.clone();
        clear_recent.connect_clicked(move |_| {
            state.borrow_mut().recent.clear();
            save_state(&state.borrow());
            render(&state);
        });
    }
    {
        let state = state.clone();
        preferences.connect_clicked(move |_| show_preferences(&state));
    }
    {
        let state = state.clone();
        refresh.connect_clicked(move |_| scan(&state));
    }
    {
        let state = state.clone();
        search.connect_search_changed(move |_| render(&state));
    }
    scan(&state);
    start_animation_clock(&state);
    window.present();
}

fn chooser(button: &gtk::Button, state: &Rc<RefCell<State>>) {
    let state = state.clone();
    button.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title("Папка со стикерами")
            .build();
        let window = state.borrow().window.clone();
        let state = state.clone();
        dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
            if let Ok(folder) = result {
                if let Some(path) = folder.path() {
                    state.borrow_mut().directory = Some(path);
                    save_state(&state.borrow());
                    scan(&state);
                }
            }
        });
    });
}

fn show_preferences(state: &Rc<RefCell<State>>) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Настройки");
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Поведение");
    let close_on_copy = adw::SwitchRow::builder()
        .title("Закрыть при копировании")
        .subtitle("Скрывать окно после помещения стикера в буфер обмена")
        .build();
    close_on_copy.set_active(state.borrow().close_on_copy);
    {
        let state = state.clone();
        close_on_copy.connect_active_notify(move |row| {
            state.borrow_mut().close_on_copy = row.is_active();
            save_state(&state.borrow());
        });
    }
    group.add(&close_on_copy);
    page.add(&group);
    dialog.add(&page);
    dialog.present(Some(&state.borrow().window));
}

fn scan(state: &Rc<RefCell<State>>) {
    let groups = state
        .borrow()
        .directory
        .as_deref()
        .map(group_stickers)
        .unwrap_or_default();
    {
        let mut s = state.borrow_mut();
        s.groups = groups;
        if s.active != RECENT_GROUP && !s.groups.contains_key(&s.active) {
            s.active = "Все".into();
        }
    }
    render_tabs(state);
    render(state);
}

fn group_stickers(root: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut groups = BTreeMap::new();
    let root_images = images(root, false);
    if !root_images.is_empty() {
        groups.insert("Корень".into(), root_images);
    }
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let list = images(&path, true);
                if !list.is_empty() {
                    groups.insert(entry.file_name().to_string_lossy().into_owned(), list);
                }
            }
        }
    }
    let mut all: Vec<PathBuf> = groups.values().flatten().cloned().collect();
    all.sort_by_key(sort_key);
    if all.is_empty() {
        BTreeMap::new()
    } else {
        let mut result = BTreeMap::new();
        result.insert("Все".into(), all);
        result.extend(groups);
        result
    }
}

fn images(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_image(&path) {
                result.push(path);
            } else if recursive && path.is_dir() {
                result.extend(images(&path, true));
            }
        }
    }
    result.sort_by_key(sort_key);
    result
}
fn sort_key(path: &PathBuf) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}
fn is_image(path: &Path) -> bool {
    path.extension()
        .is_some_and(|x| EXTENSIONS.iter().any(|e| x.eq_ignore_ascii_case(e)))
}

fn render_tabs(state: &Rc<RefCell<State>>) {
    let (tabs, groups, active) = {
        let s = state.borrow();
        (s.tabs.clone(), s.groups.clone(), s.active.clone())
    };
    while let Some(child) = tabs.first_child() {
        tabs.remove(&child);
    }
    let mut previous: Option<gtk::ToggleButton> = None;
    for (name, list) in groups {
        let button = gtk::ToggleButton::with_label(&format!("{name} ({})", list.len()));
        if let Some(previous) = &previous {
            button.set_group(Some(previous));
        }
        button.set_active(name == active);
        let state = state.clone();
        button.connect_toggled(move |button| {
            if button.is_active() && state.borrow().active != name {
                state.borrow_mut().active = name.clone();
                render(&state);
            }
        });
        previous = Some(button.clone());
        tabs.append(&button);
    }
}

fn render(state: &Rc<RefCell<State>>) {
    let (flow, stack, empty, count, window, active, shown, total, directory) = {
        let s = state.borrow();
        let query = s.search.text().trim().to_lowercase();
        let list = if s.active == RECENT_GROUP {
            s.recent.clone()
        } else {
            s.groups.get(&s.active).cloned().unwrap_or_default()
        };
        let shown: Vec<_> = list
            .iter()
            .filter(|p| {
                p.file_stem()
                    .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        (
            s.flow.clone(),
            s.stack.clone(),
            s.empty.clone(),
            s.count.clone(),
            s.window.clone(),
            s.active.clone(),
            shown,
            list.len(),
            s.directory.clone(),
        )
    };
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }
    {
        let mut s = state.borrow_mut();
        s.animations.clear();
        s.render_generation += 1;
    }
    let visible_count = shown.len();
    if shown.is_empty() {
        if total > 0 {
            empty.set_title("Ничего не найдено");
            empty.set_description(Some("Попробуйте изменить поисковый запрос."));
        } else if active == RECENT_GROUP {
            empty.set_title("Нет недавних стикеров");
            empty.set_description(Some("Скопированные стикеры появятся здесь."));
        } else if directory.is_some() {
            empty.set_title("В этой папке нет стикеров");
            empty.set_description(Some("Добавьте изображения или выберите другую папку."));
        }
        stack.set_visible_child_name("empty");
    } else {
        stack.set_visible_child_name("stickers");
        queue_sticker_buttons(state, shown);
    }
    let location = directory
        .as_ref()
        .map_or_else(|| "папка не выбрана".into(), |p| p.display().to_string());
    count.set_text(&format!(
        "{active}: {visible_count} из {total} стикеров  ·  {location}"
    ));
    let suffix = directory
        .and_then(|p| p.file_name().map(|n| format!(" — {}", n.to_string_lossy())))
        .unwrap_or_default();
    window.set_title(Some(&format!("Mem Board{suffix}")));
    state
        .borrow()
        .clear_recent
        .set_visible(active == RECENT_GROUP);
}

/// Build a small batch per idle iteration. Opening a large folder no longer
/// blocks GTK while every thumbnail and animated-image decoder is constructed.
fn queue_sticker_buttons(state: &Rc<RefCell<State>>, shown: Vec<PathBuf>) {
    let generation = state.borrow().render_generation;
    let pending = Rc::new(RefCell::new(VecDeque::from(shown)));
    let weak_state = Rc::downgrade(state);
    glib::idle_add_local(move || {
        let Some(state) = weak_state.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if state.borrow().render_generation != generation {
            return glib::ControlFlow::Break;
        }
        for _ in 0..12 {
            let Some(path) = pending.borrow_mut().pop_front() else {
                return glib::ControlFlow::Break;
            };
            let button = image_button(path, &state);
            state.borrow().flow.insert(&button, -1);
        }
        glib::ControlFlow::Continue
    });
}

fn image_button(path: PathBuf, state: &Rc<RefCell<State>>) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    let label = path
        .file_name()
        .map_or_else(|| "стикер".into(), |n| n.to_string_lossy().into_owned());
    button.set_tooltip_text(Some(&format!("{label}\nНажмите, чтобы скопировать")));
    let picture = animated_picture(&path, state).unwrap_or_else(|| {
        let picture = gtk::Picture::new();
        picture.set_file(Some(&gio::File::for_path(&path)));
        picture
    });
    picture.set_size_request(96, 96);
    picture.set_content_fit(gtk::ContentFit::Contain);
    button.set_child(Some(&picture));
    let state = state.clone();
    button.connect_clicked(move |_| copy(&state, &path));
    button
}

/// GDK textures are static. Decode GIF/WebP through GdkPixbuf and replace the
/// paintable as frames advance, so stickers animate in the thumbnail grid too.
#[allow(deprecated)] // GdkPixbuf is the GTK decoder that exposes animated image frames.
fn animated_picture(path: &Path, state: &Rc<RefCell<State>>) -> Option<gtk::Picture> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if extension != "gif" && extension != "webp" {
        return None;
    }
    let animation = gdk_pixbuf::PixbufAnimation::from_file(path).ok()?;
    let picture = gtk::Picture::new();
    let iterator = animation.iter(None);
    picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&iterator.pixbuf())));

    if animation.is_static_image() {
        return Some(picture);
    }

    state.borrow_mut().animations.push(Animation {
        picture: picture.downgrade(),
        iterator,
    });
    Some(picture)
}

/// One clock advances all visible animations. This avoids one 30fps source for
/// every GIF/WebP thumbnail, which was the main source of UI stutter.
#[allow(deprecated)] // See animated_picture: Pixbuf provides the animated frames.
fn start_animation_clock(state: &Rc<RefCell<State>>) {
    let weak_state = Rc::downgrade(state);
    glib::timeout_add_local(std::time::Duration::from_millis(33), move || {
        let Some(state) = weak_state.upgrade() else {
            return glib::ControlFlow::Break;
        };
        state.borrow_mut().animations.retain_mut(|animation| {
            let Some(picture) = animation.picture.upgrade() else {
                return false;
            };
            if animation.iterator.advance(SystemTime::now()) {
                picture.set_paintable(Some(&gdk::Texture::for_pixbuf(
                    &animation.iterator.pixbuf(),
                )));
            }
            true
        });
        glib::ControlFlow::Continue
    });
}

fn copy(state: &Rc<RefCell<State>>, path: &Path) {
    match gdk::Texture::from_file(&gio::File::for_path(path)) {
        Ok(texture) => {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set(&texture.to_value());
                let (close_on_copy, window) = {
                    let mut state = state.borrow_mut();
                    state.clipboard_texture = Some(texture);
                    state.recent.retain(|sticker| sticker != path);
                    state.recent.insert(0, path.to_path_buf());
                    state.recent.truncate(RECENT_LIMIT);
                    save_state(&state);
                    (state.close_on_copy, state.window.clone())
                };
                let name = path
                    .file_stem()
                    .map_or_else(|| "Стикер".into(), |n| n.to_string_lossy().into_owned());
                state.borrow().toasts.add_toast(adw::Toast::new(&format!(
                    "«{name}» скопирован в буфер обмена"
                )));
                // Hiding feels like closing to the user, while retaining the
                // Wayland clipboard owner until the pasted data is requested.
                if close_on_copy {
                    window.set_visible(false);
                }
            }
        }
        Err(error) => state
            .borrow()
            .toasts
            .add_toast(adw::Toast::new(&format!("Не удалось скопировать: {error}"))),
    }
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("mem-board/settings.json"))
}
fn load_settings() -> Settings {
    config_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}
fn save_state(state: &State) {
    if let Some(path) = config_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&Settings {
            sticker_directory: state.directory.clone(),
            recent_stickers: state.recent.clone(),
            close_on_copy: state.close_on_copy,
        }) {
            let _ = fs::write(path, text);
        }
    }
}
