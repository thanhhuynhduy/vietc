#!/usr/bin/python3
import signal
import sys

try:
    from PyQt6.QtCore import Qt
    from PyQt6.QtWidgets import QApplication, QDialog, QLineEdit, QVBoxLayout

    MODALITY = Qt.WindowModality.ApplicationModal
    STAY_ON_TOP = Qt.WindowType.WindowStaysOnTopHint
    FOCUS_REASON = Qt.FocusReason.OtherFocusReason
    EXEC_METHOD = "exec"
except ImportError:
    from PyQt5.QtCore import Qt
    from PyQt5.QtWidgets import QApplication, QDialog, QLineEdit, QVBoxLayout

    MODALITY = Qt.ApplicationModal
    STAY_ON_TOP = Qt.WindowStaysOnTopHint
    FOCUS_REASON = Qt.OtherFocusReason
    EXEC_METHOD = "exec_"


def main() -> int:
    app = QApplication(sys.argv)
    app.setApplicationName("Vietc controlled Qt runtime")

    dialog = QDialog()
    dialog.setObjectName("dboard-controlled-qt-dialog")
    dialog.setWindowTitle("Vietc controlled Qt runtime")
    dialog.setWindowModality(MODALITY)
    dialog.setWindowFlag(STAY_ON_TOP, True)

    field = QLineEdit(dialog)
    field.setObjectName("dboard-controlled-qt-text-field")
    field.setPlaceholderText("Controlled fixture text only")
    field.setMinimumWidth(480)

    layout = QVBoxLayout(dialog)
    layout.addWidget(field)

    def submit() -> None:
        print(field.text(), flush=True)
        dialog.accept()

    field.returnPressed.connect(submit)
    dialog.show()
    dialog.raise_()
    dialog.activateWindow()
    field.setFocus(FOCUS_REASON)
    field.grabKeyboard()

    signal.signal(signal.SIGTERM, lambda *_: dialog.reject())
    getattr(dialog, EXEC_METHOD)()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())