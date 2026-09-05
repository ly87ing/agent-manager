"""Static constants for the US Google browser-state sync."""
from __future__ import annotations

import datetime


ACCEPT_LANGUAGES = "en-US,en"
APP_LOCALE = "en-US"
COUNTRY_ID_US = 244
ENV_LOCALE = "en_US.UTF-8"
NCR_URL = "https://www.google.com/ncr"
GOOGLE_HOME_URL = "https://www.google.com/?hl=en-US&gl=US&pws=0"
SEARCH_URL = "https://www.google.com/search?q={searchTerms}&hl=en-US&gl=US&pws=0"
SUGGEST_URL = "https://www.google.com/complete/search?client=chrome&q={searchTerms}&hl=en-US&gl=US"
TRANSLATE_TARGET_LANGUAGE = "en"
VARIATIONS_COUNTRY = "us"
DEFAULT_TIMEZONE_ID = "America/Los_Angeles"
DEFAULT_GEOLOCATION = {
    "latitude": 37.7749,
    "longitude": -122.4194,
    "accuracy": 100,
}
US_TIMEZONE_PROBE_UTC = datetime.datetime(2026, 5, 22, 12, 0, 0, tzinfo=datetime.timezone.utc)
US_DESKTOP_VIEWPORT = {"width": 1365, "height": 768}
US_DESKTOP_CHROME_WINDOW = {"width": 1365, "height": 907}
# headless 探针没有窗口装饰；探针命令带 --disable-blink-features=AutomationControlled，
# 该 flag 会让 headless Chrome 额外模拟 143px 的窗口装饰高度，所以要观测到 768 的
# 视口，探针窗口高度需用 768 + 143（与有头模式的 907 常数不同，勿合并）。
US_DESKTOP_HEADLESS_PROBE_WINDOW = {
    "width": US_DESKTOP_VIEWPORT["width"],
    "height": US_DESKTOP_VIEWPORT["height"] + 143,
}
US_DESKTOP_SCREEN = {"width": 1365, "height": 768}
US_DESKTOP_SCREEN_INFO = f"{US_DESKTOP_SCREEN['width']}x{US_DESKTOP_SCREEN['height']}"
US_DESKTOP_SCREEN_COLOR_DEPTH = 24
US_DESKTOP_SCREEN_ORIENTATION_TYPE = "landscape-primary"
US_DESKTOP_SCREEN_ORIENTATION_ANGLE = 0
US_DESKTOP_DEVICE_SCALE_FACTOR = 1
US_DESKTOP_IS_MOBILE = False
US_DESKTOP_HAS_TOUCH = False
US_DESKTOP_COLOR_SCHEME = "light"
US_DESKTOP_HARDWARE_CONCURRENCY = 8
US_DESKTOP_DEVICE_MEMORY_GB = 8
US_DESKTOP_NETWORK_INFORMATION = {
    "effective_type": "4g",
    "rtt_ms": 50,
    "downlink_mbps": 10,
    "save_data": False,
    "type": None,
}
US_DESKTOP_PDF_PLUGIN_NAMES = (
    "PDF Viewer",
    "Chrome PDF Viewer",
    "Chromium PDF Viewer",
    "Microsoft Edge PDF Viewer",
    "WebKit built-in PDF",
)
US_DESKTOP_PDF_MIME_TYPES = ("application/pdf", "text/pdf")
US_DESKTOP_PDF_PLUGIN_FILENAME = "internal-pdf-viewer"
US_DESKTOP_PDF_DESCRIPTION = "Portable Document Format"
US_DESKTOP_PDF_SUFFIXES = "pdf"
US_DESKTOP_WEBGL_VENDOR = "Intel Inc."
US_DESKTOP_WEBGL_RENDERER = "Intel Iris OpenGL Engine"
US_CANVAS_FONT_METRIC_FALLBACK_FAMILY = "Arial"
US_KEYBOARD_LAYOUT_MAP = {
    "Backquote": "`",
    "Backslash": "\\",
    "BracketLeft": "[",
    "BracketRight": "]",
    "Comma": ",",
    "Digit1": "1",
    "Digit2": "2",
    "Digit3": "3",
    "Digit4": "4",
    "Digit5": "5",
    "Digit6": "6",
    "Digit7": "7",
    "Digit8": "8",
    "Digit9": "9",
    "Digit0": "0",
    "Equal": "=",
    "IntlBackslash": "\\",
    "KeyQ": "q",
    "KeyW": "w",
    "KeyY": "y",
    "KeyZ": "z",
    "Minus": "-",
    "Period": ".",
    "Quote": "'",
    "Semicolon": ";",
    "Slash": "/",
}
BLOCKED_FONT_FAMILIES = (
    "PingFang SC",
    "PingFang TC",
    "Hiragino Sans GB",
    "Heiti SC",
    "Songti SC",
    "Microsoft YaHei",
    "SimSun",
    "MingLiU",
    "PMingLiU",
    "Noto Sans CJK SC",
    "Noto Sans CJK TC",
)
EXIT_PROFILE_GEOLOCATION_ACCURACY = 50000
VERIFICATION_REPORT_VERSION = 1
FALLBACK_CHROME_VERSION = "120.0.0.0"
DISABLED_CHROME_FEATURES = (
    "ReduceAcceptLanguage",
    "WebGPU",
    "FontAccess",
    "WebBluetooth",
    "WebUSB",
    "WebHID",
    "WebXR",
    "Serial",
)
BASE_CHROME_ARGS = [
    "--lang=en-US",
    "--accept-lang=en-US,en",
    # 模板要求 light 配色；宿主机处于深色模式时 prefers-color-scheme 会跟随 OS，
    # 这里强制 light，保证有头/无头与探针观测到的指纹面一致。
    "--blink-settings=preferredColorScheme=1",
    f"--window-size={US_DESKTOP_CHROME_WINDOW['width']},{US_DESKTOP_CHROME_WINDOW['height']}",
    f"--screen-info={{{US_DESKTOP_SCREEN_INFO}}}",
    f"--force-device-scale-factor={US_DESKTOP_DEVICE_SCALE_FACTOR}",
    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
    "--disable-quic",
    "--dns-prefetch-disable",
    "--no-first-run",
    "--disable-search-engine-choice-screen",
    "--disable-sync",
    "--disable-extensions",
    "--disable-speech-api",
    "--deny-permission-prompts",
    f"--disable-features={','.join(DISABLED_CHROME_FEATURES)}",
    "--disable-blink-features=AutomationControlled",
]
BLOCK_CONTENT_SETTING = 2
DEVICE_PERMISSION_CONTENT_SETTINGS = ("media_stream_camera", "media_stream_mic", "notifications")
GOOGLE_HK_MARKERS = ("google.com.hk", "hong kong")
DENIED_PERMISSION_PROBES = (
    "camera",
    "microphone",
    "notifications",
    "clipboard-read",
    "clipboard-write",
    "idle-detection",
    "persistent-storage",
    "background-sync",
    "periodic-background-sync",
    "payment-handler",
    "midi",
    "accelerometer",
    "gyroscope",
    "magnetometer",
    "window-management",
)
SYSTEM_PROFILE_NAMES = {"System Profile", "Guest Profile"}
MANAGED_PROFILE_VOLATILE_STATE_RELATIVE_PATHS = (
    "Default/Account Web Data",
    "Default/Account Web Data-journal",
    "Default/Affiliation Database",
    "Default/Affiliation Database-journal",
    "Default/ClientCertificates",
    "Default/Cookies",
    "Default/Cookies-journal",
    "Default/DawnGraphiteCache",
    "Default/DawnWebGPUCache",
    "Default/Favicons",
    "Default/Favicons-journal",
    "Default/History",
    "Default/History-journal",
    "Default/IndexedDB",
    "Default/Login Data",
    "Default/Login Data-journal",
    "Default/Login Data For Account",
    "Default/Login Data For Account-journal",
    "Default/Local Storage",
    "Default/Network Persistent State",
    "Default/PersistentOriginTrials",
    "Default/Reporting and NEL",
    "Default/Reporting and NEL-journal",
    "Default/Safe Browsing Cookies",
    "Default/Safe Browsing Cookies-journal",
    "Default/ServerCertificate",
    "Default/ServerCertificate-journal",
    "Default/Sessions",
    "Default/Session Storage",
    "Default/Service Worker",
    "Default/Shared Dictionary",
    "Default/Site Characteristics Database",
    "Default/Top Sites",
    "Default/Top Sites-journal",
    "Default/Trust Tokens",
    "Default/Trust Tokens-journal",
    "Default/Cache",
    "Default/Code Cache",
    "Default/GPUCache",
    "Default/Web Data",
    "Default/Web Data-journal",
    "Default/WebStorage",
    "Default/Network/Cookies",
    "Default/Network/Cookies-journal",
)

