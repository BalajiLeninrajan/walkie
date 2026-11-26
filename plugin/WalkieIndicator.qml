import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Common
import qs.Services
import qs.DCommon.Widgets
import qs.Modules.Plugins

// Shows a pill near the bottom of the screen while walkie records or transcribes.
// walkie streams its state records to a socket this plugin listens on. The
// state file is read once at startup, in case DMS restarted mid-dictation.
PluginComponent {
    id: root

    readonly property string statePath: {
        const custom = (pluginData.statePath || "").trim();
        if (!custom)
            return Quickshell.env("XDG_RUNTIME_DIR") + "/walkie/state";
        return custom.startsWith("~/") ? Quickshell.env("HOME") + custom.slice(1) : custom;
    }
    readonly property int bottomMargin: pluginData.bottomMargin ?? 64
    readonly property bool focusedOnly: pluginData.screens === "focused"

    // The last record read from the state file.
    property string phase: "idle"
    property real level: 0
    property bool locked: false
    // Set when a non-idle state stops being rewritten, which means the session died.
    property bool stale: false
    property bool versionWarned: false

    readonly property bool shown: !stale && phase !== "idle"

    // Recent levels, newest first. The meter draws them from the middle out.
    readonly property int historyLength: 4
    property var history: [0, 0, 0, 0]
    property real lastShift: 0
    // Loudest level since the meter last moved.
    property real pendingPeak: 0

    function applyRecord(text) {
        const fields = {};
        for (const part of text.trim().split(/\s+/)) {
            const eq = part.indexOf("=");
            if (eq > 0)
                fields[part.slice(0, eq)] = part.slice(eq + 1);
        }
        if (Object.keys(fields).length === 0) {
            // Created but not written yet.
            phase = "idle";
            return;
        }
        if (fields.v !== "1") {
            if (!versionWarned) {
                console.warn(`walkie: state format v${fields.v ?? "(missing)"} not supported by this plugin; update one of them`);
                versionWarned = true;
            }
            phase = "idle";
            return;
        }

        const state = fields.state;
        phase = state === "recording" || state === "transcribing" ? state : "idle";
        locked = fields.locked === "1";
        level = phase === "recording" ? Math.max(0, Math.min(1, parseFloat(fields.level) || 0)) : 0;
        stale = false;

        if (phase === "idle") {
            staleTimer.stop();
            history = [0, 0, 0, 0];
            return;
        }
        staleTimer.restart();

        // The meter moves at most every 70 ms (about every other record).
        // Each repaint costs DMS CPU, and 12 updates a second still read as live.
        if (phase !== "recording")
            return;
        pendingPeak = Math.max(pendingPeak, level);
        const now = Date.now();
        if (now - lastShift >= 70) {
            // Treat levels under 0.2 (-48 dBFS) as silence and stretch the rest
            // over the bar height, so normal speech fills most of it.
            const shown = Math.max(0, Math.min(1, (pendingPeak - 0.2) / 0.6));
            history = [shown].concat(history.slice(0, historyLength - 1));
            lastShift = now;
            pendingPeak = 0;
        }
    }

    // Live updates: walkie connects here at the start of each dictation and
    // sends every state record as one line. Reading a socket costs far less
    // than reloading a FileView 25 times a second.
    SocketServer {
        active: true
        path: Quickshell.env("XDG_RUNTIME_DIR") + "/walkie-indicator.sock"
        handler: Socket {
            parser: SplitParser {
                onRead: line => root.applyRecord(line)
            }
        }
    }

    // Read once, not watched: re-reading on every change costs DMS several
    // times more CPU than the stream. A missing file just means idle.
    FileView {
        path: root.statePath
        printErrors: false
        blockLoading: true
        onLoaded: root.applyRecord(text())
        onLoadFailed: err => root.phase = "idle"
    }

    // walkie rewrites the file every 250 ms or faster during a session.
    Timer {
        id: staleTimer
        interval: 2000
        onTriggered: root.stale = true
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: win

            required property var modelData
            readonly property string focusedName: root.focusedOnly ? CompositorService.getFocusedScreenName() : ""
            readonly property bool presented: root.shown && (!focusedName || focusedName === modelData.name)

            readonly property real shadowPad: Theme.elevationRenderPadding(Theme.elevationLevel2, Theme.elevationLightDirection, Theme.spacingXS, Theme.spacingS, Theme.spacingL)
            readonly property real pillHeight: 40
            readonly property real maxPillWidth: 220

            screen: modelData
            // No surface exists while hidden. The timer keeps it up for the fade out.
            visible: presented || closeTimer.running
            onPresentedChanged: {
                if (!presented)
                    closeTimer.restart();
            }

            WlrLayershell.namespace: "dms:plugins:walkie"
            WlrLayershell.layer: WlrLayer.Overlay
            WlrLayershell.exclusiveZone: -1
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

            anchors.bottom: true
            WlrLayershell.margins {
                bottom: Math.max(0, root.bottomMargin - win.shadowPad)
            }
            implicitWidth: maxPillWidth + shadowPad * 2
            implicitHeight: pillHeight + shadowPad * 2
            color: "transparent"
            // Click-through: an empty input region.
            mask: Region {}

            Timer {
                id: closeTimer
                interval: Theme.shortDuration + 50
            }

            Item {
                id: pill

                readonly property bool transcribing: root.phase === "transcribing"
                readonly property real padding: Theme.spacingS
                readonly property real contentWidth: transcribing ? transcribingRow.implicitWidth : recordingRow.implicitWidth

                anchors.horizontalCenter: parent.horizontalCenter
                y: win.shadowPad
                width: Math.min(win.maxPillWidth, Math.round(contentWidth + padding * 2 + Theme.spacingXS))
                height: win.pillHeight
                transformOrigin: Item.Bottom
                opacity: win.presented ? 1 : 0
                scale: win.presented ? 1 : 0.85

                Behavior on opacity {
                    NumberAnimation {
                        duration: Theme.shortDuration
                        easing.type: Theme.emphasizedEasing
                    }
                }
                Behavior on scale {
                    NumberAnimation {
                        duration: Theme.shortDuration
                        easing.type: Theme.emphasizedEasing
                    }
                }
                Behavior on width {
                    NumberAnimation {
                        duration: Theme.shortDuration
                        easing.type: Theme.emphasizedEasing
                    }
                }

                ElevationShadow {
                    anchors.fill: parent
                    level: Theme.elevationLevel2
                    fallbackOffset: Theme.spacingXS
                    targetRadius: height / 2
                    targetColor: Theme.withAlpha(Theme.surfaceContainer, Theme.popupTransparency)
                    borderColor: root.locked && !pill.transcribing ? Theme.withAlpha(Theme.primary, 0.55) : Theme.outlineMedium
                    borderWidth: 1
                    shadowEnabled: Theme.elevationEnabled && SettingsData.popoutElevationEnabled
                }

                // Recording: a badge with the mic (or a lock once a tap locked it) and the level meter.
                Row {
                    id: recordingRow

                    anchors.left: parent.left
                    anchors.leftMargin: pill.padding
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: Theme.spacingS
                    opacity: pill.transcribing ? 0 : 1
                    visible: opacity > 0

                    Behavior on opacity {
                        NumberAnimation {
                            duration: Theme.shortDuration
                        }
                    }

                    Rectangle {
                        width: 26
                        height: 26
                        radius: 13
                        anchors.verticalCenter: parent.verticalCenter
                        color: Theme.withAlpha(root.locked ? Theme.primary : Theme.error, 0.18)

                        Behavior on color {
                            ColorAnimation {
                                duration: Theme.shortDuration
                            }
                        }

                        DIcon {
                            anchors.centerIn: parent
                            name: root.locked ? "lock" : "mic"
                            size: 16
                            filled: true
                            color: root.locked ? Theme.primary : Theme.error
                        }
                    }

                    Row {
                        id: meter

                        // Bars taper toward the ends; the middle bar shows the newest level.
                        readonly property var weights: [0.45, 0.7, 0.9, 1.0, 0.9, 0.7, 0.45]
                        readonly property real minBar: 6
                        readonly property real maxBar: 24

                        anchors.verticalCenter: parent.verticalCenter
                        height: maxBar
                        spacing: 3
                        rightPadding: Theme.spacingXS

                        Repeater {
                            model: meter.weights.length

                            Rectangle {
                                required property int index
                                readonly property real value: root.history[Math.abs(index - 3)] * meter.weights[index]

                                anchors.verticalCenter: parent.verticalCenter
                                width: 4
                                height: meter.minBar + (meter.maxBar - meter.minBar) * value
                                radius: 2
                                color: Theme.primary
                                // No height animation: each frame costs DMS CPU, so
                                // the bars repaint only when walkie sends a level.
                            }
                        }
                    }
                }

                // Transcribing: three dots lit in turn, and a label. The step timer
                // repaints a few times a second instead of animating every frame.
                Row {
                    id: transcribingRow

                    anchors.left: parent.left
                    anchors.leftMargin: pill.padding
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: Theme.spacingS
                    opacity: pill.transcribing ? 1 : 0
                    visible: opacity > 0

                    Behavior on opacity {
                        NumberAnimation {
                            duration: Theme.shortDuration
                        }
                    }

                    Rectangle {
                        width: 26
                        height: 26
                        radius: 13
                        anchors.verticalCenter: parent.verticalCenter
                        color: Theme.withAlpha(Theme.primary, 0.18)

                        Row {
                            id: dots

                            property int step: 0

                            anchors.centerIn: parent
                            spacing: 2

                            Timer {
                                interval: 280
                                repeat: true
                                running: win.visible && pill.transcribing
                                onTriggered: dots.step = (dots.step + 1) % 4
                                onRunningChanged: dots.step = 0
                            }

                            Repeater {
                                model: 3

                                Rectangle {
                                    required property int index
                                    readonly property bool lit: dots.step === index + 1

                                    width: 5
                                    height: 5
                                    radius: 2.5
                                    color: Theme.primary
                                    opacity: lit ? 1 : 0.4
                                    transform: Translate {
                                        y: lit ? -2.5 : 0
                                    }
                                }
                            }
                        }
                    }

                    StyledText {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "Transcribing"
                        color: Theme.surfaceText
                        font.pixelSize: Theme.fontSizeMedium
                        rightPadding: Theme.spacingXS
                    }
                }
            }
        }
    }
}
