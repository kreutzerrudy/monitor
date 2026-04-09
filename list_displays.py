import ctypes
import ctypes.wintypes as wt


class DISPLAY_DEVICE(ctypes.Structure):
    _fields_ = [
        ("cb",           wt.DWORD),
        ("DeviceName",   ctypes.c_wchar * 32),
        ("DeviceString", ctypes.c_wchar * 128),
        ("StateFlags",   wt.DWORD),
        ("DeviceID",     ctypes.c_wchar * 128),
        ("DeviceKey",    ctypes.c_wchar * 128),
    ]


DISPLAY_DEVICE_ATTACHED_TO_DESKTOP = 0x00000001


if __name__ == "__main__":
    user32 = ctypes.windll.user32
    i = 0
    while True:
        adapter = DISPLAY_DEVICE()
        adapter.cb = ctypes.sizeof(adapter)
        if not user32.EnumDisplayDevicesW(None, i, ctypes.byref(adapter), 0):
            break

        active = bool(adapter.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP)
        print(f"\nAdapter {i} ({'ACTIVE' if active else 'inactive'})")
        print(f"  DeviceName:   {adapter.DeviceName}")
        print(f"  DeviceString: {adapter.DeviceString}")
        print(f"  DeviceID:     {adapter.DeviceID}")

        monitor = DISPLAY_DEVICE()
        monitor.cb = ctypes.sizeof(monitor)
        if user32.EnumDisplayDevicesW(adapter.DeviceName, 0, ctypes.byref(monitor), 0):
            print(f"  Monitor DeviceString: {monitor.DeviceString}")
            print(f"  Monitor DeviceID:     {monitor.DeviceID}")

        i += 1
