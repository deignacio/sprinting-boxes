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

 - Using OpenCV, extract frames from the video file
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


## License

See the [LICENSE](LICENSE) file for details.
