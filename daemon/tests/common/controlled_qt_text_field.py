#!/usr/bin/python3
import signal
import sys

from PyQt5.QtCore import Qt
from PyQt5.QtWidgets import QApplication, QDialog, QLineEdit, QVBoxLayout


def main() -> int:
    app = QApplication(sys.argv)
    app.setApplicationName("Vietc controlled Qt runtime")

    dialog = QDialog()
    dialog.setObjectName("vietc-controlled-qt-dialog")
    dialog.setWindowTitle("Vietc controlled Qt runtime")
    dialog.setWindowModality(Qt.ApplicationModal)
    dialog.setWindowFlag(Qt.WindowStaysOnTopHint, True)

    field = QLineEdit(dialog)
    field.setObjectName("vietc-controlled-qt-text-field")
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
    field.setFocus(Qt.OtherFocusReason)
    field.grabKeyboard()

    signal.signal(signal.SIGTERM, lambda *_: dialog.reject())
    dialog.exec_()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())