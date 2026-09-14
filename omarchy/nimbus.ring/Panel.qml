import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

// Nimbus in the Omarchy bar.
//
// The macOS app puts all of this behind a menu bar item; this is the same menu,
// in the place Omarchy keeps such things. It owns no state of its own: every
// value is read back from `nimbus-wayland status` and every change goes out as
// `nimbus-wayland set`, so the panel and the ring can never disagree about what
// is switched on.
Panel {
  id: root
  moduleName: "nimbus.ring"
  ipcTarget: "nimbus.ring"

  // --- state, all of it mirrored from the daemon -----------------------------
  property bool present: false          // is a nimbus running at all
  property bool enabled: false
  property bool showing: false          // is a ring on screen this instant
  property string paletteName: ""
  property var settings: ({})
  property int cursor: 0
  // Which palette row is open, if any. A list of ten is too long to cycle
  // through a click at a time, and too long to leave permanently expanded in a
  // bar popup, so it opens on demand.
  // The name of the swatch under the pointer, shown on the Colour row so a
  // grid of unlabelled colours still tells you what you are about to pick.
  property string hoveredPalette: ""
  property var palettes: []

  // The bar sizes a widget from its root item, and a bare Item has no implicit
  // size of its own — without these the widget loads cleanly, reports zero
  // width, and simply never appears.
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // Lit when a ring is actually on screen, dim when Nimbus is off or has
  // nothing to ring. The bar icon answers "is it working", not "is it running".
  readonly property color barIconColor: (present && showing)
    ? (bar ? bar.barForeground : Color.foreground)
    : Qt.darker(bar ? bar.barForeground : Color.foreground, 1.55)

  readonly property var rows: [
    { key: "enabled",      label: "Enabled",     kind: "toggle" },
    { key: "__colour",     label: "Colour",      kind: "colours" },
    { key: "idle_intensity", label: "Brightness", kind: "choice",
      options: [0.18, 0.30, 0.50], titles: ["Subtle", "Normal", "Loud"] },
    { key: "band_width",   label: "Ring width",  kind: "choice",
      options: ["thin", "normal", "thick"], titles: ["Thin", "Normal", "Thick"] },
    { key: "motion_speed", label: "Motion",      kind: "choice",
      options: ["calm", "normal", "lively"], titles: ["Calm", "Normal", "Lively"] },
    { key: "turbulence",   label: "Style",       kind: "choice",
      options: ["smooth", "normal", "churny"], titles: ["Smooth", "Normal", "Churny"] },
    { key: "frame_rate",   label: "Frame rate",  kind: "choice",
      options: [15, 30, 60], titles: ["15 fps", "30 fps", "60 fps"] },
    { key: "palette_interval", label: "New colour every", kind: "choice",
      options: [0, 600, 1800, 3600], titles: ["Never", "10 min", "30 min", "1 hour"] },
    { key: "idle_threshold", label: "When you're away", kind: "choice",
      options: [0, 120, 300, 600], titles: ["Never idle", "After 2 min", "After 5 min", "After 10 min"] },
    { key: "idle_behavior", label: "…then",          kind: "choice",
      options: ["freeze", "always_animate", "fade_out"],
      titles: ["Hold still", "Keep animating", "Hide the ring"] },
    { key: "hide_in_fullscreen", label: "Hide in full screen", kind: "toggle" },
    { key: "hide_while_dragging", label: "Hide while dragging", kind: "toggle" }
  ]

  function valueFor(key) { return settings ? settings[key] : undefined }

  function titleFor(row) {
    if (row.kind === "toggle") return valueFor(row.key) ? "On" : "Off"
    if (row.kind === "colours") return hoveredPalette !== "" ? hoveredPalette : paletteName
    var v = valueFor(row.key)
    for (var i = 0; i < row.options.length; i++) {
      // Compared loosely on purpose: JSON hands back 0.3 for a float the config
      // holds as 0.30, and 1800 vs 1800.0 for the interval.
      if (Math.abs(Number(row.options[i]) - Number(v)) < 0.001) return row.titles[i]
      if (String(row.options[i]) === String(v)) return row.titles[i]
    }
    return String(v === undefined ? "—" : v)
  }

  // Step a setting to its next value. One click cycles, which suits a short
  // list far better than a submenu and keeps the whole panel one screen tall.
  function advance(row, direction) {
    // The Colour row itself steps to the next palette; the grid below is for
    // picking one directly.
    if (row.kind === "colours") { run(["next-color"]); return }
    if (row.kind === "toggle") { apply(row.key, valueFor(row.key) ? "false" : "true"); return }

    var v = valueFor(row.key)
    var at = 0
    for (var i = 0; i < row.options.length; i++) {
      if (Math.abs(Number(row.options[i]) - Number(v)) < 0.001
          || String(row.options[i]) === String(v)) { at = i; break }
    }
    var next = (at + (direction < 0 ? row.options.length - 1 : 1)) % row.options.length
    apply(row.key, String(row.options[next]))
  }

  function apply(key, value) { run(["set", key, value]) }

  function chooseColour(name) { run(["color", name]) }

  // Take a palette in or out of the timer's rotation without switching to it.
  // The macOS menu keeps this on a separate "In rotation…" submenu; here it is
  // a right-click on the same row, so the list stays one screen tall.
  function toggleRotation(name) {
    var keep = []
    for (var i = 0; i < palettes.length; i++) {
      var p = palettes[i]
      var off = !p.in_rotation
      if (p.name === name) off = !off
      if (off) keep.push(p.name)
    }
    run(["set", "disabled_palettes", keep.join(",")])
  }

  // --- talking to the daemon -------------------------------------------------

  function refresh() { if (!status.running) status.running = true }

  function adopt(text) {
    var parsed
    try { parsed = JSON.parse(text) } catch (e) { root.present = false; return }
    if (!parsed || parsed.ok !== true) { root.present = false; return }
    root.present = true
    root.showing = parsed.showing === true
    root.paletteName = parsed.palette_name || ""
    root.palettes = parsed.palettes || []
    root.settings = parsed.settings || ({})
    root.enabled = root.settings.enabled === true
  }

  Process {
    id: status
    command: ["nimbus-wayland", "status"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.adopt(String(text || ""))
    }
    // A non-zero exit means no daemon is listening, which is a state the panel
    // shows rather than an error it reports.
    onExited: function(code) { if (code !== 0) root.present = false }
  }

  // Separate from `status` so a command and a poll cannot collide on one
  // Process; the command's own reply is the fresh status, so there is no
  // round trip wasted re-asking.
  Process {
    id: command
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.adopt(String(text || ""))
    }
  }

  function run(args) {
    if (command.running) return
    command.command = ["nimbus-wayland"].concat(args)
    command.running = true
  }

  Timer {
    // Polls only while the panel is open. A bar widget that shells out every
    // few seconds forever would be a poor neighbour on a laptop.
    interval: Math.max(1, root.setting("refreshIntervalSec", 5)) * 1000
    running: root.opened
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  // The bar icon still has to be right before the panel is ever opened.
  Timer { interval: 1500; running: true; repeat: false; onTriggered: root.refresh() }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    iconComponent: Component {
      Item {
        NimbusMark {
          anchors.centerIn: parent
          iconSize: Style.space(17)
          color: root.barIconColor
        }
      }
    }
    tooltipText: root.present
      ? (root.enabled ? "Nimbus · " + root.paletteName : "Nimbus · off")
      : "Nimbus is not running"
    onPressed: function(b) {
      if (b === Qt.RightButton && root.present) { root.apply("enabled", root.enabled ? "false" : "true"); return }
      root.refresh()
      root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keys
    contentWidth: panel.fittedContentWidth(Style.space(340))
    contentHeight: panel.fittedContentHeight(column.implicitHeight)

    PanelKeyCatcher {
      id: keys
      anchors.fill: parent
      onMoveRequested: function(dx, dy) {
        if (dy !== 0) root.cursor = (root.cursor + dy + root.rows.length) % root.rows.length
        else if (dx !== 0) root.advance(root.rows[root.cursor], dx)
      }
      onActivateRequested: root.advance(root.rows[root.cursor], 1)
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Style.space(10)

        // ---------- Hero ----------
        Item {
          width: parent.width
          implicitHeight: Math.max(heroIcon.implicitHeight, heroText.implicitHeight)

          NimbusMark {
            id: heroIcon
            iconSize: Style.font.displayLarge
            color: root.showing ? root.foreground : root.dim
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            Behavior on color { ColorAnimation { duration: 200 } }
          }

          Column {
            id: heroText
            anchors.left: heroIcon.right
            anchors.leftMargin: Style.space(14)
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(2)

            Text {
              text: "Nimbus"
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.title
              font.bold: true
              width: parent.width
              elide: Text.ElideRight
            }
            Text {
              text: !root.present ? "NOT RUNNING"
                  : !root.enabled ? "OFF"
                  : root.showing ? ("RINGING · " + root.paletteName.toUpperCase())
                  : "NOTHING FOCUSED"
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.bold: true
              font.letterSpacing: 1.2
              width: parent.width
              elide: Text.ElideRight
            }
          }
        }

        PanelSeparator { width: parent.width }

        // ---------- When nothing is listening ----------
        Text {
          visible: !root.present
          width: parent.width
          wrapMode: Text.WordWrap
          text: "Start it with:\n  nimbus-wayland &\n\nTo start it with every session, add this to\n~/.config/hypr/autostart.lua:\n  o.launch_on_start(\"nimbus-wayland\")"
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
        }

        // ---------- Settings ----------
        Repeater {
          model: root.present ? root.rows : []

          Column {
            id: rowItem
            required property int index
            required property var modelData
            width: column.width
            spacing: Style.space(6)

          Rectangle {
            width: parent.width
            implicitHeight: Style.space(30)
            radius: Style.space(6)
            color: rowItem.index === root.cursor
              ? Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.10)
              : "transparent"

            Text {
              anchors.left: parent.left
              anchors.leftMargin: Style.space(8)
              anchors.verticalCenter: parent.verticalCenter
              text: rowItem.modelData.label
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.body
            }

            Text {
              anchors.right: parent.right
              anchors.rightMargin: Style.space(8)
              anchors.verticalCenter: parent.verticalCenter
              text: root.titleFor(rowItem.modelData)
              color: rowItem.modelData.kind === "toggle" && !root.valueFor(rowItem.modelData.key)
                ? root.dim : root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.body
              font.bold: true
            }

            MouseArea {
              anchors.fill: parent
              acceptedButtons: Qt.LeftButton | Qt.RightButton
              hoverEnabled: true
              onEntered: root.cursor = rowItem.index
              // Right-click steps backwards, so a four-option list is never
              // more than one click away in either direction.
              onClicked: function(e) {
                root.cursor = rowItem.index
                root.advance(rowItem.modelData, e.button === Qt.RightButton ? -1 : 1)
              }
            }
          }

            // ---------- The palettes ----------
            //
            // A grid, not a list. Ten colours stacked one per row is most of a
            // screen for information that fits in two, and it forced the settings
            // above to be hidden while it was open — which took away the controls
            // to make room for something that did not need the room.
            //
            // Swatches rather than names: the names are evocative but nobody can
            // recall what "Toxic" looks like, and the point of choosing a colour is
            // seeing it. Each tile is the palette's own two band colours with its
            // glow beneath, which is what the ring is made of.
            Grid {
              id: swatches
              visible: rowItem.modelData.kind === "colours"
              width: parent.width
              columns: 5
              spacing: Style.space(6)

              Repeater {
                model: root.palettes

                Rectangle {
                  required property var modelData
                  readonly property bool isCurrent: modelData.name === root.paletteName

                  width: (swatches.width - swatches.spacing * (swatches.columns - 1)) / swatches.columns
                  height: Style.space(26)
                  radius: Style.space(5)
                  color: "transparent"

                  // The current palette is ringed rather than ticked: a mark drawn
                  // on top would cover the colour you are trying to look at.
                  border.width: isCurrent ? Style.space(2) : (hover.containsMouse ? Style.space(1) : 0)
                  border.color: isCurrent ? root.foreground : Qt.darker(root.foreground, 1.6)

                  Item {
                    anchors.fill: parent
                    anchors.margins: Style.space(4)
                    // Out of rotation shows faded but stays clickable — hiding it
                    // would make the change impossible to undo from here.
                    opacity: modelData.in_rotation ? 1.0 : 0.30

                    Rectangle {
                      anchors.fill: parent
                      anchors.bottomMargin: Style.space(3)
                      radius: Style.space(2)
                      gradient: Gradient {
                        orientation: Gradient.Horizontal
                        GradientStop { position: 0.0; color: modelData.a }
                        GradientStop { position: 1.0; color: modelData.b }
                      }
                    }
                    Rectangle {
                      anchors.left: parent.left
                      anchors.right: parent.right
                      anchors.bottom: parent.bottom
                      height: Style.space(2)
                      radius: height / 2
                      color: modelData.glow
                    }
                  }

                  MouseArea {
                    id: hover
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    // Naming the colour under the pointer, in the Colour row above,
                    // is what makes a wordless grid legible.
                    onEntered: root.hoveredPalette = modelData.name
                    onExited: if (root.hoveredPalette === modelData.name) root.hoveredPalette = ""
                    onClicked: function(e) {
                      if (e.button === Qt.RightButton) root.toggleRotation(modelData.name)
                      else root.chooseColour(modelData.name)
                    }
                  }
                }
              }
            }
          }
        }



        PanelSeparator { width: parent.width; visible: root.present }

        Text {
          visible: root.present
          width: parent.width
          wrapMode: Text.WordWrap
          text: "Click a colour to use it · right-click to take it out of rotation\nClick a setting to change it · right-click to step back\nSaved to ~/.config/nimbus/config.json"
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
        }
      }
    }
  }
}
