# Nadex Application Documentation

This document provides a comprehensive overview of the Nadex application's architecture, components, and key workflows. It is intended for developers who want to understand, maintain, or extend the codebase.

For a user-focused guide on features and how to run the application, please see the [README.md](./README.md).

## Table of Contents

1.  [High-Level Architecture](#1-high-level-architecture)
    -   [Core Concepts: `AppState` and `AppAction`](#core-concepts-appstate-and-appaction)
    -   [The Main Event Loop (`NadexApp::update`)](#the-main-event-loop-nadexappupdate)
    -   [Service-Oriented Design](#service-oriented-design)
2.  [Service Layer Deep Dive](#2-service-layer-deep-dive)
    -   [`ImageService`](#imageservice)
    -   [`PersistenceService`](#persistenceservice)
    -   [`ThumbnailService`](#thumbnailservice)
    -   [`Updater`](#updater)
3.  [Asynchronous Operations](#3-asynchronous-operations)
    -   [Image Upload Workflow](#image-upload-workflow)
    -   [Thumbnail Generation Workflow](#thumbnail-generation-workflow)
4.  [UI Components](#4-ui-components)
    -   [Component Structure](#component-structure)
    -   [Communication via Action Queue](#communication-via-action-queue)
5.  [Data Model](#5-data-model)
    -   [`ImageManifest`](#imagemanifest)
    -   [`ImageMeta` and `MapMeta`](#imagemeta-and-mapmeta)
6.  [Configuration and Data Storage](#6-configuration-and-data-storage)

---

## 1. High-Level Architecture

The application follows a centralized state management pattern, inspired by architectures like Redux, but adapted for immediate mode GUI (`egui`). The core idea is to have a single source of truth for the application's state and to manage changes through a queue of explicit actions.

### Core Concepts: `AppState` and `AppAction`

-   **`AppState` (`src/app_state.rs`):** This struct is the single source of truth. It holds all the data required to render the UI and manage the application's state, such as the `ImageManifest`, the currently selected map, UI component states (e.g., `show_upload_modal`), and thumbnail caches. All rendering logic reads from this struct.

-   **`AppAction` (`src/app_actions.rs`):** This enum defines all possible state mutations that can occur in the application. Actions can be triggered by user interaction with the UI (e.g., `AppAction::ShowUploadModal`) or by background tasks completing (e.g., `AppAction::UploadSucceededBackgroundTask`). UI components do not modify `AppState` directly; instead, they push actions onto a queue.

### The Main Event Loop (`NadexApp::update`)

The `update` method in `NadexApp` (`src/main.rs`) is the heart of the application, executed on every frame. Its responsibilities are:

1.  **Process Actions:** It iterates through the `action_queue`, processing each `AppAction` and mutating `AppState` accordingly.
2.  **Run UI Code:** It calls the `show` methods for all visible UI components, passing them a reference to `AppState` (for rendering) and the `action_queue` (for them to push new actions).
3.  **Manage Background Tasks:** It checks for results from ongoing background tasks (like uploads) and pushes corresponding `AppAction`s onto the queue.

This one-way data flow (UI -> Action -> State -> UI) makes the application's logic predictable and easier to debug.

### Service-Oriented Design

Business logic is decoupled from the UI and main application loop by encapsulating it in services within the `src/services/` directory. This separation of concerns makes the code more modular and testable.

-   **UI Layer (`src/ui/`):** Responsible only for drawing and capturing user input.
-   **Application Layer (`src/main.rs`):** Orchestrates the flow of data between the UI and the services.
-   **Service Layer (`src/services/`):** Handles complex logic, file system operations, and background tasks.

---

## 2. Service Layer Deep Dive

The service layer contains the core business logic of the application.

### `ImageService`

Orchestrates high-level, image-related operations. It acts as a facade, coordinating other services to perform complex tasks.

-   **`orchestrate_full_upload_process`:** Manages the entire image upload flow, from validating dimensions to spawning background threads for file copying and manifest saving.
-   **`delete_image`:** Handles the complete deletion of an image, including removing its files, thumbnails, and manifest entry.

### `PersistenceService`

Handles all direct interactions with the file system for non-thumbnail image files and the JSON manifest.

-   **`save_manifest` / `load_manifest`:** Serializes/deserializes the `ImageManifest` to/from `manifest.json`.
-   **`copy_image_to_data`:** Copies a user's selected image to the application's data directory, ensuring it has a unique filename to prevent collisions.
-   **`delete_image_and_thumbnails`:** Deletes the main image file and its associated thumbnails from disk, using `ThumbnailService` to find the correct thumbnail files.

### `ThumbnailService`

Responsible for WebP image conversion and optimization.

-   **`convert_to_full_webp`:** Converts an image file to a full-size WebP format for optimal quality and performance.
-   **`request_image_conversion`:** The entry point for the asynchronous image conversion process. It receives a path to a newly uploaded image, spawns a worker thread, and converts it to WebP format.
-   **`remove_webp_images_for_image`:** Deletes all WebP images associated with a given main image.
-   **`convert_existing_images_to_webp`:** A utility function that converts existing uploaded images to WebP format on application startup to maintain compatibility.

### `Updater`

Manages automatic updates from GitHub releases.

-   **`check_for_update`:** Checks GitHub releases for new versions by comparing with the current application version using semantic versioning.
-   **`update_to_latest`:** Downloads and installs the latest version of the application.
-   **`UpdateStatus`:** An enum that represents different states of the update process (UpToDate, UpdateAvailable, Updated, Error).

---

## 3. Asynchronous Operations

To keep the UI responsive, long-running tasks like file I/O are performed on background threads.

### Image Upload Workflow

1.  **UI:** `UploadModal` pushes `AppAction::SubmitUpload` with the image path and metadata.
2.  **`NadexApp`:** The handler for `SubmitUpload` calls `ImageService::orchestrate_full_upload_process`.
3.  **`ImageService`:** Spawns a background thread to perform the upload.
    -   The thread calls `PersistenceService::copy_image_to_data` to save the file.
    -   It then creates the `ImageMeta` for the new image.
    -   On success, it sends `AppAction::UploadSucceededBackgroundTask` with the new `ImageMeta`.
    -   On failure, it sends `AppAction::UploadFailed`.
4.  **`NadexApp`:** Receives the action from the service's thread and updates `AppState` (adds the new image to the manifest, shows a notification).
5.  **Manifest Save:** The `UploadSucceededBackgroundTask` handler then triggers a *second* background task to save the updated `ImageManifest` to disk, ensuring the UI isn't blocked by this final write.

### Thumbnail Generation Workflow

1.  **`NadexApp`:** The handler for `AppAction::UploadSucceededBackgroundTask` calls `ThumbnailService::request_thumbnail_generation`.
2.  **`ThumbnailService`:** Spawns a dedicated worker thread to handle all thumbnail generation jobs.
3.  **Worker Thread:** The worker loads the full-size image, resizes it, encodes it as a WebP, and saves it to the `.thumbnails` directory.
4.  **UI Update:** The UI's `ThumbnailCache` will load these thumbnails on-demand when they are required for display in the image grid.

---

## 4. UI Components

### Component Structure

The UI is broken down into modules in the `src/ui/` directory (e.g., `image_grid_view.rs`, `top_bar_view.rs`). Each module typically exposes a single `show_...` function that takes `&mut AppState`, `&mut egui::Ui`, and `&mut Vec<AppAction>` as arguments.

### Communication via Action Queue

UI components are stateless from `egui`'s perspective. They are re-drawn from scratch on every frame based on the data in `AppState`. When a user interacts with a widget (e.g., clicks a button), the component does not change any state itself. Instead, it pushes an `AppAction` onto the `action_queue`. The main `update` loop will process this action on the next frame, which will in turn update `AppState` and cause the UI to re-render to reflect the new state.

---

## 5. Data Model

Core data structures are defined in `src/persistence.rs`.

### `ImageManifest`

This is the main data structure that is serialized to `manifest.json`. It acts as the database for the application.

-   **`images`:** A `HashMap` where the key is a map name (e.g., "Mirage") and the value is a `Vec<ImageMeta>` for all images on that map.
-   **`maps`:** A `HashMap` storing metadata for each map, such as the `MapMeta` containing the last accessed time.

### `ImageMeta` and `MapMeta`

-   **`ImageMeta`:** Contains all information about a single lineup image, including its unique `filename`, `map_name`, `nade_type`, `position`, `notes`, and creation `timestamp`.
-   **`MapMeta`:** Stores metadata about a map, currently just the `last_accessed` timestamp to allow sorting maps by recent use.

---

## 6. Configuration and Data Storage

The application stores all its data in a `nadex` folder within the user's standard local data directory. The location varies by OS:

-   **Windows:** `C:\Users\<YourUser>\AppData\Local\nadex`
-   **Linux:** `/home/<YourUser>/.local/share/nadex`
-   **macOS:** `/Users/<YourUser>/Library/Application Support/nadex`

This directory contains:
-   `manifest.json`: The central database file.
-   A sub-directory for each map (e.g., `Mirage/`), containing the full-size image files.
-   A `.thumbnails` directory within each map folder, containing the generated WebP thumbnails.
