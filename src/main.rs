
// Mandatory COSMIC imports
use cosmic::app::Core;
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::iced::advanced::widget;
use cosmic::iced::futures::SinkExt;
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
#[derive(Default)]
struct Window {
    core: Core,
    popup: Option<window::Id>,
    sys: sysinfo::System,
    free: u64,
    total: u64,
    standard_model: segmented_button::SingleSelectModel,
    prefix: usize,
    update_interval_tx: watch::Sender<u64>,
    update_interval_text: String,
    precision: u32,
    enable_show_total: bool,
}

#[derive(Clone, Debug)]
enum Message {
    Tick, // Triggered on a user-defined interval
    TogglePopup, // Mandatory for open and close the applet
    PopupClosed(window::Id), // Mandatory for the applet to know if it's been closed
    UpdateStandard(Entity), // The user changed the standard in which byte counts are presented
    UpdatePrecision(u32), // The user adjusted the precision of the byte counts
    UpdatePrefix(usize), // The user changed the prefix with which byte counts are presented
    UpdateInterval(String), // The user changed the interval with which the data is updated
    UpdateShowTotal(bool), // The user toggled whether to show total RAM
    Surface(surface::Action), // Actions that should be re-routed to COSMIC
}

static AUTOSIZE_MAIN_ID: Lazy<widget::Id> = Lazy::new(|| widget::Id::new("autosize-main"));

impl Window {

    fn standard(&self) -> Standard {
        *self.standard_model.active_data().unwrap()
    }

    fn refresh(&mut self) {
        self.sys.refresh_memory();
        self.free = self.sys.used_memory();
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
        let _si_entity = standard_model
            .insert()
            .text("SI")
            .data(Standard::Si)
            .id();
        let iec_entity = standard_model
            .insert()
            .data(Standard::Iec)
            .text("IEC")
            .id();
        standard_model.activate(iec_entity);

        let mut window = Window {
            core, // Set the incoming core
            sys: System::new(),
            standard_model,
            prefix: 0,
            update_interval_tx: watch::Sender::new(DEFAULT_UPDATE_INTERVAL),
            update_interval_text: DEFAULT_UPDATE_INTERVAL.to_string(),
            ..Default::default() // Set everything else to the default values
        };

        // Immediately load statistics when the application loads
        window.refresh();

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
                                #[cfg(debug_assertions)]
                                if let Err(err) = output.send(Message::Tick).await {
                                    tracing::error!(?err, "Failed sending tick request to applet");
                                }
                                #[cfg(not(debug_assertions))]
                                let _ = output.send(Message::Tick).await;
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
        time_subscription(show_seconds_rx)
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
            // Unset the popup field after it's been closed
            Message::PopupClosed(popup_id) => {
                if self.popup.as_ref() == Some(&popup_id) {
                    self.popup = None;
                }
            }
            Message::UpdatePrecision(prec) => {
                self.precision = prec;
            }
            Message::UpdateStandard(entity) => {
                self.standard_model.activate(entity);
                self.refresh();
            }
            Message::Tick => {
                self.refresh();
            }
            Message::UpdateShowTotal(enable) => {
                self.enable_show_total = enable;
            }
            Message::Surface(a) => return cosmic::task::message(cosmic::Action::Cosmic(
                cosmic::app::Action::Surface(a)
            )), // FIXME No idea what this should do
            // Update the prefix with which the bytes are displayed
            Message::UpdatePrefix(prefix) => self.prefix = prefix,
            Message::UpdateInterval(text) => {
                if let Ok(msec) = text.parse::<u64>() {
                    if msec > 0 {
                        // Don't panic if the update could not be processed
                        let _ = self.update_interval_tx.send(msec);
                    }
                }
                self.update_interval_text = text;
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

        let prefix = match self.prefix {
            0 => Prefix::Auto,
            1 => Prefix::None,
            2 => Prefix::Kilo,
            3 => Prefix::Mega,
            4 => Prefix::Giga,
            5 => Prefix::Tera,
            // 6 => Prefix::Peta,
            // 7 => Prefix::Exa,
            // 8 => Prefix::Zeta,
            // 9 => Prefix::Yotta,
            _ => unreachable!(),
        };
        let icon = button::icon(icon::from_name("display-symbolic"))
            .on_press(Message::TogglePopup);
        let usage = self.core.applet.text(format_bytes(self.free, self.standard(), prefix, self.precision));
        let mut children = vec![
            Element::from(icon), Element::from(usage)
        ];
        if self.enable_show_total {
            let total = self.core.applet.text(format_bytes(self.total, self.standard(), prefix, self.precision));
            children.push(Element::from(self.core.applet.text(" / ")));
            children.push(Element::from(total));
        }
        let button = button::custom(
            if horizontal {
                Element::from(cosmic::widget::row::with_children(children))
            } else {
                Element::from(cosmic::widget::column::with_children(children))
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
        let content_list = column![
            settings::item(
                "Update Interval (in ms)",
                text_input("", &self.update_interval_text)
                    .on_input(Message::UpdateInterval),
            ),
            settings::item(
                "Standard",
                segmented_control::horizontal(&self.standard_model)
                    .on_activate(Message::UpdateStandard),
            ),
            settings::item(
                "Prefix",
                popup_dropdown(
                    &PREFIX_MENU_ITEMS,
                    Some(self.prefix),
                    Message::UpdatePrefix,
                    self.popup.unwrap_or(window::Id::RESERVED),
                    Message::Surface,
                    |a| a,
                )
            ),
            settings::item(
                "Precision",
                spin_button(
                    format!("{}", self.precision),
                    self.precision,
                    1,
                    0,
                    10,
                    Message::UpdatePrecision,
                ),
            ),
            settings::item(
                "Show Total",
                checkbox("", self.enable_show_total)
                    .on_toggle(Message::UpdateShowTotal)
            ),
        ]
        .padding(5)
        .spacing(0);

        // Set the widget content list as the popup_container for the applet
        self.core
            .applet
            .popup_container(container(content_list))
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

#[derive(Default, Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum Standard {
    #[default]
    Si,
    Iec,
}

#[derive(Default, Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
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

