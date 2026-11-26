import QtQuick
import qs.Common
import qs.Modules.Plugins

PluginSettings {
    pluginId: "walkie"

    SliderSetting {
        settingKey: "bottomMargin"
        label: "Bottom margin"
        description: "Distance from the bottom edge of the screen to the pill."
        defaultValue: 64
        minimum: 0
        maximum: 400
        unit: "px"
    }

    SelectionSetting {
        settingKey: "screens"
        label: "Screens"
        description: "Show the pill on every screen, or only on the one with focus."
        options: [
            {
                "label": "All screens",
                "value": "all"
            },
            {
                "label": "Focused screen",
                "value": "focused"
            }
        ]
        defaultValue: "all"
    }

    StringSetting {
        settingKey: "statePath"
        label: "State file"
        description: "The file walkie writes its state to, read once when the plugin starts. Leave empty for $XDG_RUNTIME_DIR/walkie/state."
        placeholder: "$XDG_RUNTIME_DIR/walkie/state"
        defaultValue: ""
    }
}
