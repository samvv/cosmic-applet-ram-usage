
// Mandatory COSMIC imports
use cosmic::app::Core;
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic::cosmic_config; // Necessary for CosmicConfigEntry derivation to work
use cosmic::cosmic_config::{Config, CosmicConfigEntry};
use cosmic::iced::advanced::widget;
use cosmic::iced::futures::SinkExt;
use cosmic::iced::Alignment::Center;
use cosmic::iced_futures::stream;
use cosmic::iced::Subscription;
use cosmic::iced::{
    platform_specific::shell::commands::popup::{destroy_popup, get_popup},
    Limits,
};
use cosmic::iced_runtime::core::window;
use cosmic::iced_widget::column;
use cosmic::widget::dropdown::popup_dropdown;
use cosmic::widget::segmented_button::{Entity, SingleSelectModel};
use cosmic::{surface, Element};
use cosmic::app::Task;

// Widgets we're going to use
use cosmic::widget::{autosize, button, checkbox, text_input, container, icon, segmented_button, segmented_control, settings, spin_button};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sysinfo::System;
use tokio::{sync::watch, time};

// Every COSMIC Application and Applet MUST have an ID
const ID: &str = "be.samvervaeck.CosmicAppletRAM";

const DEFAULT_UPDATE_INTERVAL: u64 = 1000;

/*
*  Every COSMIC model must be a struct data type.
*  Mandatory fields for a COSMIC Applet are core and popup.
*  Core is the core settings that allow it to interact with COSMIC
*  and popup, as you'll see later, is the field that allows us to open
*  and close the applet.
*
*  Next we have our custom field that we will manipulate the value of based
*  on the message we send.
*/
struct Window {
    core: Core,
    popup: Option<window::Id>,
    sys: sysinfo::System,
    used: u64,
    total: u64,
    standard_model: segmented_button::SingleSelectModel,
    entity_si: Entity,
    entity_iec: Entity,
    update_interval_tx: watch::Sender<u64>,
    live_config: CosmicAppletRamConfig,
    config: Config,
    // Exclusively UI state
    update_interval_text: String,
}

const VERSION: u64 = 1;

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize, Clone, CosmicConfigEntry)]
#[version = 1]
struct CosmicAppletRamConfig {
    precision: u32,
    prefix: Prefix,
    show_total: bool,
    standard: Standard,
    update_interval: u64,
}

impl Default for CosmicAppletRamConfig {
    fn default() -> Self {
        Self {
            precision: 0,
            prefix: Prefix::Auto,
            show_total: true,
            standard: Standard::Iec,
            update_interval: DEFAULT_UPDATE_INTERVAL,
        }
    }
}

#[derive(Clone, Debug)]
enum Message {
    Tick, // Triggered on a user-defined interval
    TogglePopup, // Mandatory for open and close the applet
    PopupClosed(window::Id), // Mandatory for the applet to know if it's been closed
    UpdateStandard(Standard), // The user changed the standard in which byte counts are presented
    UpdatePrecision(u32), // The user adjusted the precision of the byte counts
    UpdatePrefix(Prefix), // The user changed the prefix with which byte counts are presented
    UpdateInterval(String), // The user changed the interval with which the data is updated
    UpdateShowTotal(bool), // The user toggled whether to show total RAM
    ConfigChanged(CosmicAppletRamConfig), // The configuration values were somehow changed
    Surface(surface::Action), // Actions that should be re-routed to COSMIC
}

trait ResultExt {
    fn log<S: AsRef<str>>(self, msg: S);
}

impl <T, E: std::fmt::Display> ResultExt for std::result::Result<T, E> {
    fn log<S: AsRef<str>>(self, msg: S) {
        if let Err(error) = self {
            tracing::error!("{}: {}", msg.as_ref(), error);
        }
    }
}


static AUTOSIZE_MAIN_ID: Lazy<widget::Id> = Lazy::new(|| widget::Id::new("autosize-main"));

impl Window {

    /// Low-level utility to change the amount of milliseconds between each tick.
    ///
    /// This method does not save configuration.
    fn set_ticks(&mut self, msec: u64) {
        // Don't panic if the update could not be processed
        let _ = self.update_interval_tx.send(msec);
    }

    /// Changes the standard with which counters are formatted.
    ///
    /// This method does not save configuration.
    fn ui_set_standard(&mut self, standard: Standard) {
        self.standard_model.activate(
            match standard {
                Standard::Si => self.entity_si,
                Standard::Iec => self.entity_iec,
            }
        );
        self.live_config.standard = standard;
    }

    /// Changes the interval at which the UI updates to the given value.
    ///
    /// This method overrides whatever value was present in the text input. The text input will be
    /// changed to reflect the given value.
    ///
    /// This method does not save configuration.
    fn ui_set_update_interval(&mut self, msec: u64) {
        self.live_config.update_interval = msec;
        self.update_interval_text = msec.to_string();
        self.set_ticks(msec);
    }

    /// Changes the prefix with which counters are displayed.
    ///
    /// This method does not save configuration.
    fn ui_set_prefix(&mut self, prefix: Prefix) {
        self.live_config.prefix = prefix;
    }

    /// Changes the precision with which counters are formatted.
    ///
    /// This method does not save configuration.
    fn ui_set_precision(&mut self, precision: u32) {
        self.live_config.precision = precision;
    }

    /// Change whether to display the total installed amount of RAM.
    ///
    /// This method does not save configuration.
    fn ui_set_show_total(&mut self, enable: bool) {
        self.live_config.show_total = enable;
    }

    /// Refresh the metrics that are rendered to the screen.
    fn refresh_metrics(&mut self) {
        self.sys.refresh_memory();
        self.used = self.sys.used_memory();
        self.total = self.sys.total_memory();
    }

}

impl cosmic::Application for Window {
    /*
    *  Executors are a mandatory thing for both COSMIC Applications and Applets.
    *  They're basically what allows for multi-threaded async operations for things that
    *  may take too long and block the thread the GUI is running on. This is also where
    *  Tasks take place.
    */
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = (); // Argument passed to init()
    type Message = Message; // These are setting the application messages to our Message enum
    const APP_ID: &'static str = ID; // This is where we set our const above to the actual ID

    // Setup the immutable core functionality.
    fn core(&self) -> &Core {
        &self.core
    }

    // Set up the mutable core functionality.
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    // Initialize the applet
    /*
    *  The parameters are the Core and flags (again not sure what to do with these).
    *  The function returns our model struct initialized and an Option<Task>, in this case
    *  there is no command so it returns a None value with the type of Task in its place.
    */
    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {

        let mut standard_model = SingleSelectModel::default();
        let entity_si = standard_model
            .insert()
            .text("SI")
            .data(Standard::Si)
            .id();
        let entity_iec = standard_model
            .insert()
            .data(Standard::Iec)
            .text("IEC")
            .id();

        let config = Config::new(ID, VERSION).expect("failed to load config for RAM usage applet");

        let live_config = CosmicAppletRamConfig::get_entry(&config).unwrap_or_default();

        let mut window = Window {
            core, // Set the incoming core
            popup: None, // No popup should be open on startup
            sys: System::new(),
            used: 0,
            total: 0,
            standard_model,
            entity_si,
            entity_iec,
            update_interval_tx: watch::Sender::new(live_config.update_interval),
            update_interval_text: live_config.update_interval.to_string(),
            live_config,
            config,
        };

        // Force the segmented control to select its initial value
        window.ui_set_standard(window.live_config.standard);

        // Immediately load statistics when the application loads
        window.refresh_metrics();

        (window, Task::none())
    }

    // Create what happens when the applet is closed
    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        // Pass the PopupClosed message to the update function
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> cosmic::iced::Subscription<Self::Message> {
        fn time_subscription(mut msec_watch: watch::Receiver<u64>) -> Subscription<Message> {
            Subscription::run_with_id(
                "time-sub",
                stream::channel(1, |mut output| async move {
                    // Mark this receiver's state as changed so that it always receives an initial
                    // update during the loop below
                    // This allows us to avoid duplicating code from the loop
                    msec_watch.mark_changed();
                    let mut msec = 1000;
                    let mut timer = time::interval(time::Duration::from_millis(msec));
                    timer.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

                    loop {
                        tokio::select! {
                            _ = timer.tick() => {
                                output.send(Message::Tick).await.log("Failed sending tick request to applet");
                            },
                            // Update timer if the user toggles show_seconds
                            Ok(()) = msec_watch.changed() => {
                                msec = *msec_watch.borrow_and_update();
                                let period = time::Duration::from_millis(msec);
                                let start = time::Instant::now() + period;
                                timer = time::interval_at(start, period);
                                timer.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
                            }
                        }
                    }
                }),
            )
        }
        let show_seconds_rx = self.update_interval_tx.subscribe();
        Subscription::batch(vec![
            self.core
                .watch_config(Self::APP_ID)
                .map(|u| {
                    for err in u.errors {
                        tracing::error!(?err, "Error watching config");
                    }
                    Message::ConfigChanged(u.config)
                }),
            time_subscription(show_seconds_rx),
        ])
    }

    // Here is the update function, it's the one that handles all of the messages that
    // are passed within the applet.
    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        // match on what message was sent
        match message {
            // Handle the TogglePopup message
            Message::TogglePopup => {
                // Close the popup
                return if let Some(popup_id) = self.popup.take() {
                    destroy_popup(popup_id)
                } else {
                    // Create and "open" the popup
                    let parent_win_id = match self.core.main_window_id() {
                        Some(id) => id,
                        // Early return if the widget window somehow wasn't present
                        None => return Task::none(),
                    };
                    let new_id = window::Id::unique();
                    self.popup.replace(new_id);

                    let mut popup_settings = self.core.applet.get_popup_settings(
                        parent_win_id,
                        new_id,
                        None,
                        None,
                        None
                    );

                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(372.0)
                        .min_width(300.0)
                        .min_height(200.0)
                        .max_height(1080.0);

                    get_popup(popup_settings)
                }
            }
            // Unset the popup field after it has been closed
            Message::PopupClosed(popup_id) => {
                if self.popup.as_ref() == Some(&popup_id) {
                    self.popup = None;
                }
            }
            Message::Tick => {
                self.refresh_metrics();
            }
            Message::UpdatePrecision(prec) => {
                self.live_config
                    .set_precision(&self.config, prec)
                    .log("Failed to save applet configuration");
                self.ui_set_precision(prec);
                self.refresh_metrics();
            }
            Message::UpdateStandard(standard) => {
                self.live_config
                    .set_standard(&self.config, standard)
                    .log("Failed to save applet configuration");
                self.ui_set_standard(standard);
                self.refresh_metrics();
            }
            Message::UpdateShowTotal(enable) => {
                self.live_config
                    .set_show_total(&self.config, enable)
                    .log("Failed to save applet configuration");
                self.ui_set_show_total(enable);
                self.refresh_metrics();
            }
            Message::UpdatePrefix(prefix) => {
                self.live_config
                    .set_prefix(&self.config, prefix)
                    .log("Failed to save applet configuration");
                self.ui_set_prefix(prefix);
                self.refresh_metrics();
            }
            Message::UpdateInterval(text) => {
                if let Ok(msec) = text.parse::<u64>() {
                    if msec > 0 {
                        self.live_config
                            .set_update_interval(&self.config, msec)
                            .log("save configuration failed");
                        self.set_ticks(msec);
                    }
                }
                self.update_interval_text = text;
            }
            Message::Surface(a) => return cosmic::task::message(cosmic::Action::Cosmic(
                cosmic::app::Action::Surface(a)
            )),
            Message::ConfigChanged(config) => {
                if config.precision != self.live_config.precision {
                    self.ui_set_precision(config.precision);
                }
                if config.prefix != self.live_config.prefix {
                    self.ui_set_prefix(config.prefix);
                }
                if config.show_total != self.live_config.show_total {
                    self.ui_set_show_total(config.show_total);
                }
                if config.standard != self.live_config.standard {
                    self.ui_set_standard(config.standard);
                }
                if config.update_interval != self.live_config.update_interval {
                    self.ui_set_update_interval(config.update_interval);
                }
            }
        }
        Task::none() // Again not doing anything that requires multi-threading here.
    }

    /*
    *  For an applet, the view function describes what an applet looks like. There's a
    *  secondary view function (view_window) that shows the widgets in the popup when it's
    *  opened.
    */
    fn view(&self) -> Element<Self::Message> {
        let horizontal = matches!(
            self.core.applet.anchor,
            PanelAnchor::Top | PanelAnchor::Bottom
        );

        let padding = self.core.applet.suggested_padding(false);
        let icon = container(icon::from_name("display-symbolic"))
            .padding(padding);
        let usage = self.core.applet.text(
            format_bytes(
                self.used,
                self.live_config.standard,
                self.live_config.prefix,
                self.live_config.precision
            )
        );
        let mut children = vec![
            Element::from(icon), Element::from(usage)
        ];
        if self.live_config.show_total {
            let total = self.core.applet.text(
                format_bytes(
                    self.total,
                    self.live_config.standard,
                    self.live_config.prefix,
                    self.live_config.precision
                )
            );
            children.push(Element::from(self.core.applet.text(" / ")));
            children.push(Element::from(total));
        }
        let button = button::custom(
            if horizontal {
                Element::from(cosmic::widget::row::with_children(children).align_y(Center))
            } else {
                Element::from(cosmic::widget::column::with_children(children).align_x(Center))
            },
        )
        .on_press_down(Message::TogglePopup)
        .class(cosmic::theme::Button::AppletIcon);

        autosize::autosize(
            button,
            AUTOSIZE_MAIN_ID.clone()
        )
        .into()
        // self.core
        //     .applet
        //     .icon_button("display-symbolic") // Using a default button image
        //     .on_press(Message::TogglePopup)
        //     .into()
    }

    // The actual GUI window for the applet. It's a popup.
    fn view_window(&self, _id: window::Id) -> Element<Self::Message> {

        // Needed to compare later on which control is selected
        let entity_iec = self.entity_iec.clone();
        let entity_si = self.entity_si.clone();

        let cosmic::cosmic_theme::Spacing {
            space_s, ..
        } = cosmic::theme::spacing();

        let content_list = column![
            settings::item(
                "Update Interval (in ms)",
                text_input("", &self.update_interval_text)
                    .on_input(Message::UpdateInterval),
            ),
            settings::item(
                "Standard",
                segmented_control::horizontal(&self.standard_model)
                    .on_activate(move |e| Message::UpdateStandard(
                        if e == entity_iec {
                            Standard::Iec
                        } else if e == entity_si {
                            Standard::Si
                        } else {
                            unreachable!()
                        }
                    ))
            ),
            settings::item(
                "Prefix",
                popup_dropdown(
                    &PREFIX_MENU_ITEMS,
                    Some(
                        match self.live_config.prefix {
                            Prefix::Auto => 0,
                            Prefix::None => 1,
                            Prefix::Kilo => 2,
                            Prefix::Mega => 3,
                            Prefix::Giga => 4,
                            Prefix::Tera => 5,
                            Prefix::Peta => 6,
                            Prefix::Exa => 7,
                            Prefix::Zeta => 8,
                            Prefix::Yotta => 9,
                        }
                    ),
                    |p| Message::UpdatePrefix(
                        match p {
                            0 => Prefix::Auto,
                            1 => Prefix::None,
                            2 => Prefix::Kilo,
                            3 => Prefix::Mega,
                            4 => Prefix::Giga,
                            5 => Prefix::Tera,
                            6 => Prefix::Peta,
                            7 => Prefix::Exa,
                            8 => Prefix::Zeta,
                            9 => Prefix::Yotta,
                            _ => unreachable!(),
                        }
                    ),
                    self.popup.unwrap_or(window::Id::RESERVED),
                    Message::Surface,
                    |a| a,
                )
            ),
            settings::item(
                "Precision",
                spin_button(
                    format!("{}", self.live_config.precision),
                    self.live_config.precision,
                    1,
                    0,
                    10,
                    Message::UpdatePrecision,
                ),
            ),
            settings::item(
                "Show Total",
                checkbox("", self.live_config.show_total)
                    .on_toggle(Message::UpdateShowTotal)
            ),
        ]
        .spacing(space_s);

        // Set the widget content list as the popup_container for the applet
        self.core
            .applet
            .popup_container(container(content_list).padding(space_s))
            .into()
    }
}

const PREFIX_MENU_ITEMS: [&str; 6] = [
    "Auto",
    "None",
    "Kilo",
    "Mega",
    "Giga",
    "Tera",
    // "Peta",
    // "Exa",
    // "Zeta",
    // "Yotta",
];

#[derive(Default, Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
enum Standard {
    #[default]
    Si,
    Iec,
}

#[derive(Default, Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
enum Prefix {
    #[default]
    Auto,
    None,
    Kilo,
    Mega,
    Giga,
    Tera,
    Peta,
    Exa,
    Zeta,
    Yotta,
}

const PREFIXES: [&str; 9] = [
    "",
    "K",
    "M",
    "G",
    "T",
    "P",
    "E",
    "Z",
    "Y"
];

fn format_bytes(count: u64, standard: Standard, prefix: Prefix, precision: u32) -> String {
    let (k, infix) = match standard {
        Standard::Si => (1000, "i"),
        Standard::Iec => (1024, ""),
    };
    let i = match prefix {
        Prefix::Auto => {
            let mut x = count;
            let mut i = 0;
            loop {
                if i > PREFIXES.len() {
                    // If the number is excessively large just display in bytes
                    break 0
                }
                if x < k {
                    break i
                }
                x /= k;
                i += 1;
            }
        },
        Prefix::None => 0,
        Prefix::Kilo => 1,
        Prefix::Mega => 2,
        Prefix::Giga => 3,
        Prefix::Tera => 4,
        Prefix::Peta => 5,
        Prefix::Exa => 6,
        Prefix::Zeta => 7,
        Prefix::Yotta => 8,
    };
    if i == 0 {
        return format!("{count} B")
    }
    let f = (count as f64) / (k.pow(i as u32) as f64);
    let prefix_str = PREFIXES[i];
    format!("{f:.prec$} {prefix_str}{infix}B", prec = precision as usize)
}

// The main function returns a cosmic::iced::Result that is returned from
// the run function that's part of the applet module.
fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Window>(())
}

