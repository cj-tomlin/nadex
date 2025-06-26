use crate::services::updater::{self, UpdateStatus};
use egui::{Button, Color32, Context, RichText, Window};
use std::{sync::mpsc, thread};

/// State for the update dialog
pub struct UpdateDialog {
    /// Whether the dialog is open
    pub open: bool,
    /// The current update status
    pub status: Option<UpdateStatus>,
    /// Receiver for startup update check results
    pub startup_check_receiver: Option<mpsc::Receiver<UpdateStatus>>,
    /// Whether an automatic update is in progress
    pub auto_updating: bool,
    /// Whether we are currently checking for updates
    pub checking: bool,
    /// Whether we are currently updating
    pub updating: bool,
}

impl Default for UpdateDialog {
    fn default() -> Self {
        Self {
            open: false,
            status: None,
            startup_check_receiver: None,
            auto_updating: false,
            checking: false,
            updating: false,
        }
    }
}

impl UpdateDialog {
    /// Check for updates in a background thread
    pub fn check_for_updates(&mut self, ctx: &Context) {
        if self.checking || self.updating {
            return;
        }

        self.checking = true;
        self.status = None;

        let ctx = ctx.clone();

        thread::spawn(move || {
            // Check for updates
            let status = updater::check_for_update();

            // Update UI on the main thread
            ctx.request_repaint();

            // Return the status
            status
        });
    }

    /// Start the update process in a background thread
    pub fn perform_update(&mut self, ctx: &Context) {
        if self.checking || self.updating {
            return;
        }

        self.updating = true;
        self.status = None;

        let ctx = ctx.clone();
        let auto_update = self.auto_updating;

        thread::spawn(move || {
            // Perform the update
            let status = updater::update_to_latest();

            // For automatic updates, if successful, restart the application
            if auto_update {
                if let UpdateStatus::Updated { .. } = &status {
                    log::info!("Auto-update: Update complete, restarting application");
                    // Give a short delay for the UI to update
                    thread::sleep(std::time::Duration::from_secs(1));

                    // Restart the application (this will exit the current process)
                    if let Err(e) = updater::restart_application() {
                        log::error!("Failed to restart application: {}", e);
                    }
                }
            }

            // Update UI on the main thread
            ctx.request_repaint();

            // Return the status
            status
        });
    }

    /// Process any auto-update check from startup and display the update dialog if needed
    pub fn show(&mut self, ctx: &Context) {
        // Check if we have any auto-update results from startup
        if let Some(receiver) = &self.startup_check_receiver {
            if let Ok(status) = receiver.try_recv() {
                match &status {
                    UpdateStatus::UpdateAvailable { version, .. } => {
                        log::info!("Auto-update: Found new version {}", version);
                        // Store the status and start updating automatically
                        self.status = Some(status);
                        self.auto_updating = true;
                        self.open = true;
                        self.perform_update(ctx);
                    }
                    UpdateStatus::UpToDate => {
                        log::info!("Auto-update: Application is up to date");
                        // No need to show the dialog
                    }
                    _ => {
                        log::info!("Auto-update: {:?}", status);
                    }
                }

                // Remove the receiver now that we've processed the result
                self.startup_check_receiver = None;
            }
        }

        // Don't show the dialog if not open
        if !self.open {
            return;
        }

        Window::new("Nadex Updates")
            .resizable(false)
            .collapsible(false)
            .min_width(400.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    // Show title
                    ui.heading("Nadex Auto-Updater");
                    ui.add_space(10.0);

                    // Show status
                    match &self.status {
                        Some(UpdateStatus::UpToDate) => {
                            ui.label(
                                RichText::new("Your application is up to date!")
                                    .color(Color32::from_rgb(0, 150, 0)),
                            );
                        }
                        Some(UpdateStatus::UpdateAvailable { version, notes }) => {
                            ui.label(
                                RichText::new(format!("New version available: {}", version))
                                    .color(Color32::from_rgb(0, 100, 200)),
                            );
                            ui.label("Release Notes:");
                            ui.label(notes);
                            ui.add_space(10.0);

                            let update_btn =
                                ui.add_enabled(!self.updating, Button::new("Update Now"));
                            if update_btn.clicked() {
                                self.perform_update(ctx);
                            }
                        }
                        Some(UpdateStatus::Updated { version }) => {
                            ui.label(
                                RichText::new(format!(
                                    "Successfully updated to version {}",
                                    version
                                ))
                                .color(Color32::from_rgb(0, 150, 0)),
                            );
                            ui.label("Please restart the application to apply the update.");
                        }
                        Some(UpdateStatus::Error(error)) => {
                            ui.label(
                                RichText::new("Update check failed:")
                                    .color(Color32::from_rgb(200, 0, 0)),
                            );
                            ui.label(error);
                        }
                        None => {
                            if self.checking {
                                ui.label("Checking for updates...");
                                // Could add a spinner here
                            } else if self.updating {
                                ui.label("Downloading and installing update...");
                                // Could add a progress bar here
                            } else {
                                ui.label("Press Check Now to look for updates");
                            }
                        }
                    }

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!self.checking && !self.updating, Button::new("Check Now"))
                            .clicked()
                        {
                            self.check_for_updates(ctx);
                        }

                        if ui.button("Close").clicked() {
                            self.open = false;
                        }
                    });
                });
            });
    }
}
