import QtQuick
import QtQuick.Shapes
import qs.Commons

// The Nimbus mark, ported from Sources/Nimbus/UI/StatusItemIcon.swift in the
// macOS project — drawn rather than shipped as an asset, for the same reason it
// is there: a bar icon is a silhouette at around 18px, so no colour or gradient
// from the actual ring can survive. Only shape does.
//
// Hence a window outline ringed by a *broken* band. The break-up is what the
// real ring looks like where the turbulence thins it, and it is also what keeps
// the mark from reading as a progress spinner.
//
// Every proportion below is the macOS one unchanged.
Item {
  id: root

  property real iconSize: Style.font.icon
  property color color: Color.foreground

  width: iconSize
  height: iconSize
  implicitWidth: iconSize
  implicitHeight: iconSize

  readonly property real s: Math.min(width, height)
  readonly property real rectW: s * 0.40
  readonly property real rectH: s * 0.30
  readonly property real rectX: width / 2 - s * 0.20
  readonly property real rectY: height / 2 - s * 0.15
  readonly property real radius: s * 0.06

  // Far enough out that the window and the band never merge into a blob at
  // small sizes — the failure mode every tighter version of this had.
  readonly property real spread: s * 0.155
  readonly property real bandStroke: s * 0.062

  Shape {
    anchors.fill: parent
    antialiasing: true
    layer.enabled: true
    layer.samples: 4

    // The window.
    ShapePath {
      fillColor: "transparent"
      strokeColor: root.color
      strokeWidth: root.s * 0.085
      joinStyle: ShapePath.RoundJoin

      PathRectangle {
        x: root.rectX
        y: root.rectY
        width: root.rectW
        height: root.rectH
        radius: root.radius
      }
    }

    // The ring around it, deliberately uneven.
    ShapePath {
      fillColor: "transparent"
      strokeColor: root.color
      strokeWidth: root.bandStroke
      capStyle: ShapePath.RoundCap
      strokeStyle: ShapePath.DashLine

      // macOS gives the dash pattern in points: [0.16, 0.115, 0.075, 0.10,
      // 0.21, 0.105] × size. Qt's dashPattern is in multiples of the stroke
      // width instead, so dividing each by the same 0.062 reproduces it exactly
      // and stays correct at every icon size.
      dashPattern: [
        0.160 / 0.062, 0.115 / 0.062, 0.075 / 0.062,
        0.100 / 0.062, 0.210 / 0.062, 0.105 / 0.062
      ]

      PathRectangle {
        x: root.rectX - root.spread
        y: root.rectY - root.spread
        width: root.rectW + root.spread * 2
        height: root.rectH + root.spread * 2
        radius: root.radius + root.spread
      }
    }
  }
}
