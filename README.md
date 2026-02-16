# Sprinting Boxes

Sprinting Boxes is a project designed for video analysis and tracking, specifically tailored for ultimate-frisbee-based heuristics.


## Heuristics

- Are teams in a "ready" to play state (enough people standing in both end zones)
- Transition / cliff detection between the pre-point ready state -> pull -> playing state
- Based on the speed of end zones emptying, inferring the team that has pulled, and thus, which team scored the last point


## Underlying workflow and components

### Game / video configuration

 - Get information about the video file (resolution, frame rate)
 - Which game format is this? (7s, 5s, 4s)
 - Where are the end zones and where is the field in the video?

### Video processing pipeline

 - Using OpenCV or FFmpeg (VideoToolbox), extract frames from the video file
 - Extract crops for the end zones (and maybe the field)
 - Run object detection on the crops, using a SAHI tactic to deal with really small players

### Feature computation

- Number of people in each end zone (and field if enabled)
  - End zone containment includes a padding into the field
- Pre-point ready state score


## Progress Dashboard

- Opens a web server to display pipeline process and audit and confirm predictions made by the pipeline
- Ensures that configuration is specified, provides UI to specify it if not
- Displays overall progress, throughput information
- Includes worker / threadpool performance to help with tuning
- Displays the last 10 frames processed
- Once the whole video is processed and confirmed, can export results as YouTube chapters, Insta360 Studio Clips XML, and other formats


## Getting Started

Follow these instructions to set up the project from a fresh macOS installation.

### Prerequisites

1.  **Install Homebrew** (if not already installed):
    ```bash
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    ```

2.  **Install System Dependencies**:
    We need `opencv` and `ffmpeg` (for video processing) and `pkg-config` (to help Rust find them).
    ```bash
    brew install opencv ffmpeg pkg-config
    ```

3.  **Install Node.js & npm** (via nvm):
    We recommend using [nvm](https://github.com/nvm-sh/nvm) to manage Node versions.
    ```bash
    # Install nvm
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
    
    # Reload shell configuration (or restart terminal)
    source ~/.zshrc  # or ~/.bash_profile
    
    # Install latest LTS Node.js version
    nvm install --lts
    nvm use --lts
    ```

4.  **Install Rust**:
    If you haven't already:
    ```bash
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    ```
    or
    ```bash
    brew install rust
    ```


### Configuration

1.  **Environment Variables**:
    Copy the example configuration file:
    ```bash
    cp .env.example .env
    ```
    
    **Edit `.env`** and specify the required paths:
    -   `SPRINTING_BOXES_VIDEO_ROOT`: Absolute path to your video files directory.
    -   `SPRINTING_BOXES_OUTPUT_ROOT`: Absolute path where analysis results will be saved.

### Running the Application

This project consists of a Rust backend and a React frontend.

#### Start the Application
This compiles the Rust backend, builds the frontend assets automatically, and starts the server.
```bash
# In the repository root
cargo run -p sprinting-boxes
```
*The server typically runs on port `12206`, and the dashboard is embedded.*

## License

See the [LICENSE](LICENSE) file for details.
