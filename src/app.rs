//! Application shell and the SDK Versions screen.

use std::time::Duration;

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, IconName, Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
        label::Label,
        scroll::ScrollableElement,
        sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    },
    *,
};

use crate::android::{self, SdkSnapshot};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    SdkVersions,
    BuildTools,
    Devices,
    Emulators,
    Settings,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::SdkVersions => "SDK Versions",
            Page::BuildTools => "Build Tools",
            Page::Devices => "Devices",
            Page::Emulators => "Emulators",
            Page::Settings => "Settings",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::SdkVersions => {
                "Android SDK platforms — installed and available, read from the Android CLI."
            }
            Page::BuildTools => {
                "Android SDK Build-Tools — installed and available, read from the Android CLI."
            }
            Page::Devices => "Devices connected through adb, including running emulators.",
            Page::Emulators => "Android Virtual Devices.",
            Page::Settings => "SDK root, Java, and CLI configuration.",
        }
    }

    fn body(self) -> &'static str {
        match self {
            Page::SdkVersions | Page::BuildTools | Page::Devices => "",
            Page::Emulators => {
                "Not implemented yet. Will list and manage AVDs with `android emulator`."
            }
            Page::Settings => {
                "Not implemented yet. Will show the resolved SDK root and CLI paths from `android info`."
            }
        }
    }

    fn icon(self) -> IconName {
        match self {
            Page::SdkVersions => IconName::HardDrive,
            Page::BuildTools => IconName::SquareTerminal,
            Page::Devices => IconName::Cpu,
            Page::Emulators => IconName::LayoutDashboard,
            Page::Settings => IconName::Settings,
        }
    }
}

enum Mutation {
    Install(String),
    Update(String),
    Remove(String),
}

impl Mutation {
    fn package(&self) -> &str {
        match self {
            Mutation::Install(package) | Mutation::Update(package) | Mutation::Remove(package) => {
                package
            }
        }
    }

    fn verb(&self) -> &'static str {
        match self {
            Mutation::Install(_) => "Installed",
            Mutation::Update(_) => "Updated",
            Mutation::Remove(_) => "Removed",
        }
    }

    fn gerund(&self) -> &'static str {
        match self {
            Mutation::Install(_) => "Installing",
            Mutation::Update(_) => "Updating",
            Mutation::Remove(_) => "Removing",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Info,
    Success,
    Error,
}

struct Status {
    kind: StatusKind,
    text: String,
}

impl Status {
    fn info(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Info,
            text: text.into(),
        }
    }

    fn success(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Success,
            text: text.into(),
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Error,
            text: text.into(),
        }
    }
}

pub struct App {
    page: Page,
    sdk: SdkSnapshot,
    search_input: Entity<InputState>,
    running: android::RunningCli,
    status: Option<Status>,
    busy: bool,
    confirm_remove: Option<String>,
}

/// A normalized row shared by the platform and build-tools tables.
struct PackageRow {
    api_level: Option<String>,
    package: String,
    version: String,
    description: String,
    installed: bool,
}

impl App {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search by package, version, or description")
        });
        cx.subscribe(&search_input, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();

        let running = android::RunningCli::default();
        let quit_running = running.clone();
        cx.on_app_quit(move |_, _| {
            let running = quit_running.clone();
            async move {
                // SIGTERM first so the CLI can remove its temp files, then
                // SIGKILL if it outlives the grace period.
                running.shutdown(Duration::from_millis(150));
            }
        })
        .detach();

        Self {
            page: Page::SdkVersions,
            sdk: android::snapshot(),
            search_input,
            running,
            status: None,
            busy: false,
            confirm_remove: None,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.sdk = android::snapshot();
        self.confirm_remove = None;
        cx.notify();
    }

    fn start_mutation(&mut self, mutation: Mutation, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(cli) = self.sdk.cli.clone() else {
            self.status = Some(Status::error("Android CLI (`android`) not found on PATH."));
            cx.notify();
            return;
        };

        let root = self.sdk.root.clone();
        let cli_process = self.running.clone();
        let package = mutation.package().to_string();
        let running = format!("{} {package}…", mutation.gerund());
        let verb = mutation.verb();
        self.busy = true;
        self.confirm_remove = None;
        self.status = Some(Status::info(running));
        cx.notify();

        let task = cx.background_spawn(async move {
            match mutation {
                Mutation::Install(package) => {
                    android::install(&cli, root.as_deref(), &package, &cli_process)
                }
                Mutation::Update(package) => {
                    android::update(&cli, root.as_deref(), &package, &cli_process)
                }
                Mutation::Remove(package) => {
                    android::remove(&cli, root.as_deref(), &package, &cli_process)
                }
            }
        });

        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.sdk = android::snapshot();
                this.status = Some(match outcome {
                    Ok(_) => Status::success(format!("{verb} {package}.")),
                    Err(err) => Status::error(err),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let weak = cx.weak_entity();

        let item = |page: Page| {
            let weak = weak.clone();
            SidebarMenuItem::new(page.title())
                .icon(page.icon())
                .active(self.page == page)
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if this.page != page {
                            this.page = page;
                            this.confirm_remove = None;
                            if page == Page::Devices {
                                this.sdk.refresh_devices();
                            }
                            cx.notify();
                        }
                    });
                })
        };

        let menu = SidebarMenu::new().children([
            item(Page::SdkVersions),
            item(Page::BuildTools),
            item(Page::Devices),
            item(Page::Emulators),
            item(Page::Settings),
        ]);

        Sidebar::new("sidebar")
            .header(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_3()
                    .child(
                        Label::new("Android CLI")
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new("Unofficial GUI")
                            .text_size(px(11.))
                            .text_color(muted),
                    ),
            )
            .footer(
                div().px_3().py_3().child(
                    Label::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .text_size(px(11.))
                        .text_color(muted),
                ),
            )
            .child(menu)
    }

    fn render_main(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .bg(cx.theme().background)
            .child(self.render_cli_info(cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_6()
                    .py_4()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                Label::new(self.page.title())
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                Label::new(self.page.subtitle())
                                    .text_size(px(12.))
                                    .text_color(muted),
                            ),
                    )
                    .child(
                        Button::new("refresh")
                            .outline()
                            .label("Refresh")
                            .icon(IconName::RefreshCw)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(self.render_page(cx)),
            )
    }

    fn render_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        match self.page {
            Page::SdkVersions => self.render_sdk_versions(cx),
            Page::BuildTools => self.render_build_tools(cx),
            Page::Devices => self.render_devices(cx),
            page => Self::render_placeholder(page, cx),
        }
    }

    fn render_sdk_versions(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (raw_query, query) = self.search_query(cx);
        let platforms = &self.sdk.platforms;
        let installed_count = platforms
            .iter()
            .filter(|platform| platform.installed)
            .count();
        let available_count = platforms.len() - installed_count;
        let mut summary = format!("{installed_count} installed · {available_count} available");
        if !query.is_empty() {
            let matching = platforms
                .iter()
                .filter(|platform| platform.matches(&query))
                .count();
            summary.push_str(&format!(" · {matching} matching"));
        }

        let rows: Vec<PackageRow> = platforms
            .iter()
            .filter(|platform| platform.matches(&query))
            .map(|platform| PackageRow {
                api_level: Some(platform.api_level.clone()),
                package: platform.package.clone(),
                version: platform.version.clone(),
                description: platform.description.clone(),
                installed: platform.installed,
            })
            .collect();

        let (empty_title, empty_detail) = if query.is_empty() {
            (
                "No SDK platforms found".to_string(),
                "`android sdk list --all` returned no platform packages.".to_string(),
            )
        } else {
            (
                format!("No SDK versions match “{raw_query}”"),
                "Try an API level, package id, or description fragment.".to_string(),
            )
        };

        self.render_package_page(rows, summary, empty_title, empty_detail, true, cx)
    }

    fn render_build_tools(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (raw_query, query) = self.search_query(cx);
        let build_tools = &self.sdk.build_tools;
        let installed_count = build_tools.iter().filter(|tool| tool.installed).count();
        let available_count = build_tools.len() - installed_count;
        let mut summary = format!("{installed_count} installed · {available_count} available");
        if !query.is_empty() {
            let matching = build_tools
                .iter()
                .filter(|tool| tool.matches(&query))
                .count();
            summary.push_str(&format!(" · {matching} matching"));
        }

        let rows: Vec<PackageRow> = build_tools
            .iter()
            .filter(|tool| tool.matches(&query))
            .map(|tool| PackageRow {
                api_level: None,
                package: tool.package.clone(),
                version: tool.version.clone(),
                description: tool.description.clone(),
                installed: tool.installed,
            })
            .collect();

        let (empty_title, empty_detail) = if query.is_empty() {
            (
                "No build-tools found".to_string(),
                "`android sdk list --all` returned no build-tools packages.".to_string(),
            )
        } else {
            (
                format!("No build-tools match “{raw_query}”"),
                "Try a version, package id, or description fragment.".to_string(),
            )
        };

        self.render_package_page(rows, summary, empty_title, empty_detail, false, cx)
    }

    fn render_devices(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.sdk.cli.is_none() {
            return self.render_cli_missing(cx);
        }

        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let total = self.sdk.devices.len();
        let online = self
            .sdk
            .devices
            .iter()
            .filter(|device| device.is_online())
            .count();
        let summary = format!("{online} online · {total} attached");

        let header = div()
            .flex()
            .items_center()
            .gap_4()
            .px_6()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(
                Label::new("DEVICE")
                    .text_size(px(10.))
                    .text_color(muted)
                    .flex_1(),
            )
            .child(
                Label::new("TYPE")
                    .text_size(px(10.))
                    .text_color(muted)
                    .w(px(90.)),
            )
            .child(
                Label::new("STATE")
                    .text_size(px(10.))
                    .text_color(muted)
                    .w(px(110.)),
            )
            .child(
                Label::new("DETAILS")
                    .text_size(px(10.))
                    .text_color(muted)
                    .flex_1(),
            );

        let mut content = div().flex().flex_col().w_full().child(
            div()
                .px_6()
                .py_2()
                .child(Label::new(summary).text_size(px(11.)).text_color(muted)),
        );

        if let Some(error) = &self.sdk.devices_error {
            content = content.child(Self::render_error(error, cx));
        }
        content = content.child(header);

        if self.sdk.devices.is_empty() {
            let (title, detail) = if self.sdk.devices_error.is_some() {
                (
                    "Device listing unavailable".to_string(),
                    "Fix the adb issue above, then click Refresh.".to_string(),
                )
            } else {
                (
                    "No devices connected".to_string(),
                    "Start an emulator with `android emulator start <name>`, or connect a \
                     device with USB debugging enabled and accept the debugging prompt."
                        .to_string(),
                )
            };
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_6()
                    .py_8()
                    .max_w(px(680.))
                    .child(
                        Label::new(title)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(detail)
                            .text_size(px(12.))
                            .text_color(muted)
                            .whitespace_normal(),
                    ),
            );
        } else {
            for (ix, device) in self.sdk.devices.iter().enumerate() {
                content = content.child(self.render_device_row(device, ix, cx));
            }
        }

        content
            .child(
                div().px_6().py_4().child(
                    Label::new(
                        "Devices and states come from `adb devices -l` using the SDK's platform-tools.",
                    )
                    .text_size(px(11.))
                    .text_color(muted),
                ),
            )
            .into_any_element()
    }

    fn render_device_row(
        &self,
        device: &android::Device,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let state = match device.state.as_str() {
            "device" => "Online",
            "unauthorized" => "Unauthorized",
            "offline" => "Offline",
            state => state,
        };
        let state_color = if device.is_online() {
            cx.theme().success
        } else if device.state == "unauthorized" || device.state == "offline" {
            cx.theme().danger
        } else {
            muted
        };
        let kind = if device.is_emulator {
            "Emulator"
        } else {
            "Device"
        };

        let mut details = Vec::new();
        if let Some(model) = &device.model {
            details.push(model.clone());
        } else if let Some(product) = &device.product {
            details.push(product.clone());
        }
        if let Some(product) = &device.product {
            if device.model.as_deref() != Some(product) {
                details.push(format!("product {product}"));
            }
        }
        if let Some(device_name) = &device.device {
            details.push(format!("device {device_name}"));
        }
        if let Some(usb) = &device.usb {
            details.push(format!("USB {usb}"));
        }
        let details = if details.is_empty() {
            "—".to_string()
        } else {
            details.join(" · ")
        };

        div()
            .flex()
            .items_center()
            .gap_4()
            .px_6()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(
                div().flex_1().min_w(px(0.)).child(
                    Label::new(device.serial.clone())
                        .id(("device-serial", ix))
                        .text_size(px(12.))
                        .truncate(),
                ),
            )
            .child(
                Label::new(kind)
                    .id(("device-kind", ix))
                    .text_size(px(12.))
                    .text_color(muted)
                    .w(px(90.)),
            )
            .child(
                Label::new(state)
                    .id(("device-state", ix))
                    .text_size(px(12.))
                    .text_color(state_color)
                    .w(px(110.)),
            )
            .child(
                div().flex_1().min_w(px(0.)).child(
                    Label::new(details)
                        .id(("device-details", ix))
                        .text_size(px(12.))
                        .text_color(muted)
                        .truncate(),
                ),
            )
    }

    fn search_query(&self, cx: &mut Context<Self>) -> (String, String) {
        let raw_query = self.search_input.read(cx).value().trim().to_string();
        let query = raw_query.to_lowercase();
        (raw_query, query)
    }

    /// Shared scaffold for the package list pages.
    fn render_package_page(
        &mut self,
        rows: Vec<PackageRow>,
        summary: String,
        empty_title: String,
        empty_detail: String,
        show_api: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.sdk.cli.is_none() {
            return self.render_cli_missing(cx);
        }

        let mut content = div().flex().flex_col().w_full();
        content = content.child(self.render_search_bar(cx));

        if let Some(status) = &self.status {
            content = content.child(self.render_status(status, cx));
        }
        if let Some(error) = &self.sdk.error {
            content = content.child(Self::render_error(error, cx));
        }

        content = content.child(self.render_package_table(
            rows,
            summary,
            empty_title,
            empty_detail,
            show_api,
            cx,
        ));
        content
            .child(
                div().px_6().pb_6().child(
                    Label::new(
                        "Values come from `android sdk list`. Install, update, and remove run the Android CLI.",
                    )
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground),
                ),
            )
            .into_any_element()
    }

    fn render_cli_missing(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let error = self
            .sdk
            .error
            .clone()
            .unwrap_or_else(|| "Android CLI not found on PATH.".to_string());

        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_3()
            .px_6()
            .py_8()
            .max_w(px(680.))
            .child(
                Label::new("Android CLI not found")
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().danger),
            )
            .child(Label::new(error).text_size(px(12.)).whitespace_normal())
            .child(
                Label::new(
                    "This app does not bundle the SDK: it reads and modifies the SDK by running \
                     the official `android` CLI. Install the CLI from \
                     developer.android.com/tools/agents/android-cli, then click Check again. \
                     On macOS the app also checks /opt/homebrew/bin and /usr/local/bin.",
                )
                .text_size(px(12.))
                .text_color(muted)
                .whitespace_normal(),
            )
            .child(
                div().pt_2().child(
                    Button::new("check-cli")
                        .primary()
                        .label("Check again")
                        .icon(IconName::RefreshCw)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
            )
            .into_any_element()
    }

    fn render_cli_info(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;

        let detail = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(Label::new(label).text_size(px(10.)).text_color(muted))
                .child(Label::new(value).text_size(px(12.)))
        };

        div()
            .flex()
            .items_start()
            .gap_8()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(border)
            .child(detail(
                "ANDROID CLI",
                match (&self.sdk.cli, &self.sdk.cli_version) {
                    (Some(_), Some(version)) => version.clone(),
                    (Some(path), None) => path.display().to_string(),
                    (None, _) => "Not found on PATH".to_string(),
                },
            ))
            .child(detail(
                "SDK",
                self.sdk
                    .root
                    .as_ref()
                    .map(|root| root.display().to_string())
                    .unwrap_or_else(|| "Unknown".to_string()),
            ))
    }

    fn render_search_bar(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        div().px_6().py_3().child(
            Input::new(&self.search_input)
                .prefix(IconName::Search)
                .cleanable(true),
        )
    }

    fn render_status(&self, status: &Status, cx: &mut Context<Self>) -> impl IntoElement {
        let color = match status.kind {
            StatusKind::Info => cx.theme().muted_foreground,
            StatusKind::Success => cx.theme().success,
            StatusKind::Error => cx.theme().danger,
        };

        div().px_6().pb_3().child(
            div().max_w(px(760.)).child(
                Label::new(status.text.clone())
                    .text_size(px(12.))
                    .text_color(color)
                    .whitespace_normal(),
            ),
        )
    }

    fn render_error(message: &str, cx: &mut Context<Self>) -> impl IntoElement {
        div().px_6().pb_3().child(
            div().max_w(px(760.)).child(
                Label::new(message.to_string())
                    .text_size(px(12.))
                    .text_color(cx.theme().danger)
                    .whitespace_normal(),
            ),
        )
    }

    fn render_package_table(
        &mut self,
        rows: Vec<PackageRow>,
        summary: String,
        empty_title: String,
        empty_detail: String,
        show_api: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;

        let mut header = div()
            .flex()
            .items_center()
            .gap_4()
            .px_6()
            .py_2()
            .border_b_1()
            .border_color(border);
        if show_api {
            header = header.child(
                Label::new("API")
                    .text_size(px(10.))
                    .text_color(muted)
                    .w(px(70.)),
            );
        }
        header = header
            .child(
                Label::new("PACKAGE")
                    .text_size(px(10.))
                    .text_color(muted)
                    .flex_1(),
            )
            .child(
                Label::new("VERSION")
                    .text_size(px(10.))
                    .text_color(muted)
                    .w(px(70.)),
            )
            .child(
                Label::new("DESCRIPTION")
                    .text_size(px(10.))
                    .text_color(muted)
                    .w(px(200.)),
            )
            .child(
                Label::new("STATE")
                    .text_size(px(10.))
                    .text_color(muted)
                    .w(px(80.)),
            )
            .child(div().w(px(160.)));

        let mut table = div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .px_6()
                    .py_2()
                    .child(Label::new(summary).text_size(px(11.)).text_color(muted)),
            )
            .child(header);

        if rows.is_empty() {
            table = table.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_6()
                    .py_8()
                    .child(
                        Label::new(empty_title)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        Label::new(empty_detail)
                            .text_size(px(12.))
                            .text_color(muted),
                    ),
            );
        } else {
            for (ix, row) in rows.iter().enumerate() {
                table = table.child(self.render_package_row(row, ix, show_api, cx));
            }
        }

        table
    }

    fn render_package_row(
        &self,
        row: &PackageRow,
        ix: usize,
        show_api: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let (state, state_color) = if row.installed {
            ("Installed", cx.theme().success)
        } else {
            ("Available", muted)
        };

        let mut element = div()
            .flex()
            .items_center()
            .gap_4()
            .px_6()
            .py_2()
            .border_b_1()
            .border_color(border);

        if show_api {
            element = element.child(
                Label::new(row.api_level.clone().unwrap_or_default())
                    .id(("api", ix))
                    .text_size(px(13.))
                    .w(px(70.)),
            );
        }

        element
            .child(
                div().flex_1().min_w(px(0.)).child(
                    Label::new(row.package.clone())
                        .id(("package", ix))
                        .text_size(px(12.))
                        .truncate(),
                ),
            )
            .child(
                Label::new(row.version.clone())
                    .id(("version", ix))
                    .text_size(px(12.))
                    .text_color(muted)
                    .w(px(70.)),
            )
            .child(
                div().w(px(200.)).min_w(px(0.)).child(
                    Label::new(row.description.clone())
                        .id(("description", ix))
                        .text_size(px(12.))
                        .text_color(muted)
                        .truncate(),
                ),
            )
            .child(
                Label::new(state)
                    .id(("state", ix))
                    .text_size(px(12.))
                    .text_color(state_color)
                    .w(px(80.)),
            )
            .child(self.render_row_actions(&row.package, row.installed, ix, cx))
    }

    fn render_row_actions(
        &self,
        package: &str,
        installed: bool,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let confirming = self.confirm_remove.as_deref() == Some(package);
        let disabled = self.busy || self.sdk.cli.is_none();
        let package = package.to_string();
        let mut actions = div().flex().items_center().gap_2().w(px(160.));

        if confirming {
            actions = actions
                .child(
                    Button::new(("confirm-remove", ix))
                        .danger()
                        .small()
                        .label("Confirm")
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_mutation(Mutation::Remove(package.clone()), cx);
                        })),
                )
                .child(
                    Button::new(("cancel-remove", ix))
                        .ghost()
                        .small()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_remove = None;
                            cx.notify();
                        })),
                );
        } else if installed {
            let update_package = package.clone();
            let remove_package = package.clone();
            actions = actions
                .child(
                    Button::new(("update", ix))
                        .ghost()
                        .small()
                        .label("Update")
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_mutation(Mutation::Update(update_package.clone()), cx);
                        })),
                )
                .child(
                    Button::new(("remove", ix))
                        .ghost()
                        .small()
                        .label("Remove")
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_remove = Some(remove_package.clone());
                            cx.notify();
                        })),
                );
        } else {
            let install_package = package.clone();
            actions = actions.child(
                Button::new(("install-row", ix))
                    .primary()
                    .small()
                    .label("Install")
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.start_mutation(Mutation::Install(install_package.clone()), cx);
                    })),
            );
        }

        actions
    }

    fn render_placeholder(page: Page, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_6()
            .py_8()
            .max_w(px(560.))
            .child(
                Label::new("Not implemented yet")
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                Label::new(page.body())
                    .text_size(px(12.))
                    .text_color(cx.theme().muted_foreground),
            )
            .into_any_element()
    }
}

impl Render for App {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .child(self.render_sidebar(cx))
            .child(self.render_main(cx))
    }
}
