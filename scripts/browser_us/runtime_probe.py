"""Extracted from sync_browser_google_us.py: runtime_probe layer."""
from __future__ import annotations

import argparse
import datetime
import http.cookiejar
import http.server
import json
import os
import re
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from browser_us.constants import *  # noqa: F401,F403
from browser_us.model import *  # noqa: F401,F403
from browser_us.helpers import *  # noqa: F401,F403
from browser_us.sync_audit import *  # noqa: F401,F403
from browser_us.chrome import *  # noqa: F401,F403
from browser_us.verify_report import *  # noqa: F401,F403


def runtime_probe_html() -> str:
    marker = json.dumps(RUNTIME_JSON_MARKER)
    return (
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body></body>"
        "<script>"
        "(async()=>{"
        "const observed={"
        "userAgent:navigator.userAgent,"
        "platform:navigator.platform,"
        "userAgentDataMobile:navigator.userAgentData?navigator.userAgentData.mobile:null,"
        "userAgentDataPlatform:navigator.userAgentData?navigator.userAgentData.platform:null,"
        "userAgentDataBrands:navigator.userAgentData?Array.from(navigator.userAgentData.brands||[]):null,"
        "userAgentDataHighEntropy:null,"
        "webdriver:navigator.webdriver,"
        "vendor:navigator.vendor,"
        "product:navigator.product,"
        "productSub:navigator.productSub,"
        "appCodeName:navigator.appCodeName,"
        "appName:navigator.appName,"
        "appVersion:navigator.appVersion,"
        "language:navigator.language,"
        "languages:Array.from(navigator.languages||[]),"
        "innerWidth:window.innerWidth,"
        "innerHeight:window.innerHeight,"
        "outerWidth:window.outerWidth,"
        "outerHeight:window.outerHeight,"
        "screenWidth:window.screen.width,"
        "screenHeight:window.screen.height,"
        "availWidth:window.screen.availWidth,"
        "availHeight:window.screen.availHeight,"
        "screenColorDepth:window.screen.colorDepth,"
        "screenPixelDepth:window.screen.pixelDepth,"
        "screenOrientationType:window.screen.orientation?window.screen.orientation.type:null,"
        "screenOrientationAngle:window.screen.orientation?window.screen.orientation.angle:null,"
        "devicePixelRatio:window.devicePixelRatio,"
        "maxTouchPoints:navigator.maxTouchPoints,"
        "coarsePointer:window.matchMedia('(pointer: coarse)').matches,"
        "finePointer:window.matchMedia('(pointer: fine)').matches,"
        "hoverHover:window.matchMedia('(hover: hover)').matches,"
        "lightColorScheme:window.matchMedia('(prefers-color-scheme: light)').matches,"
        "darkColorScheme:window.matchMedia('(prefers-color-scheme: dark)').matches,"
        "pdfViewerEnabled:navigator.pdfViewerEnabled,"
        "pluginsLength:navigator.plugins.length,"
        "mimeTypesLength:navigator.mimeTypes.length,"
        "plugins:Array.from(navigator.plugins||[]).map((plugin)=>({"
        "name:plugin.name,filename:plugin.filename,description:plugin.description,length:plugin.length,"
        "types:Array.from(plugin).map((mimeType)=>({type:mimeType.type,suffixes:mimeType.suffixes,"
        "description:mimeType.description}))"
        "})),"
        "mimeTypes:Array.from(navigator.mimeTypes||[]).map((mimeType)=>({type:mimeType.type,"
        "suffixes:mimeType.suffixes,description:mimeType.description,"
        "enabledPlugin:mimeType.enabledPlugin?mimeType.enabledPlugin.name:null})),"
        "speechSynthesisPresent:'speechSynthesis' in window,"
        "localFontAccessPresent:'queryLocalFonts' in window,"
        "bluetoothPresent:'bluetooth' in navigator,"
        "usbPresent:'usb' in navigator,"
        "hidPresent:'hid' in navigator,"
        "serialPresent:'serial' in navigator,"
        "xrPresent:'xr' in navigator,"
        "gamepadCount:typeof navigator.getGamepads==='function'?"
        "Array.from(navigator.getGamepads()||[]).filter(Boolean).length:0,"
        "timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,"
        "intlLocale:Intl.DateTimeFormat().resolvedOptions().locale,"
        "intlCalendar:Intl.DateTimeFormat().resolvedOptions().calendar,"
        "intlNumberingSystem:Intl.DateTimeFormat().resolvedOptions().numberingSystem,"
        "intlHourCycle:Intl.DateTimeFormat(undefined,{hour:'numeric'}).resolvedOptions().hourCycle,"
        "collatorLocale:Intl.Collator().resolvedOptions().locale,"
        "pluralRulesLocale:new Intl.PluralRules().resolvedOptions().locale,"
        "relativeTimeFormatLocale:new Intl.RelativeTimeFormat().resolvedOptions().locale,"
        "intlLocaleRegion:null,"
        "navigatorLanguageRegion:null,"
        "localeWeekInfo:null,"
        "localeTextInfo:null,"
        "localeHourCycles:null,"
        "localeCalendars:null,"
        "localeNumberingSystems:null,"
        "localeTimeZones:null,"
        "numberFormat:new Intl.NumberFormat().format(1234567.89),"
        "currencyFormat:new Intl.NumberFormat(undefined,{style:'currency',currency:'USD'}).format(1234.56),"
        "dateFormat:new Intl.DateTimeFormat(undefined,{dateStyle:'short',timeZone:'UTC'}).format(new Date(Date.UTC(2026,4,22))),"
        "dateToString:new Date(Date.UTC(2026,4,22,12,0,0)).toString(),"
        "dateToTimeString:new Date(Date.UTC(2026,4,22,12,0,0)).toTimeString(),"
        "timeZoneLongFormat:null,"
        "timeZoneNameLong:null,"
        "regionDisplayName:null,"
        "listFormat:new Intl.ListFormat().format(['A','B','C']),"
        "timezoneOffset:new Date().getTimezoneOffset(),"
        "geolocationPermission:null,"
        "notificationPermission:typeof Notification==='function'?Notification.permission:null,"
        "connectionPresent:!!(navigator.connection||navigator.mozConnection||navigator.webkitConnection),"
        "connectionEffectiveType:null,"
        "connectionSaveData:null,"
        "connectionType:null,"
        "webrtcSupported:typeof RTCPeerConnection==='function',"
        "webrtcCandidateCount:0,"
        "webrtcHostCandidateCount:0,"
        "webrtcMdnsCandidateCount:0,"
        "webrtcPrivateIpCandidateCount:0,"
        "webrtcLocalIpLeak:false"
        "};"
        "try{observed.intlLocaleRegion=new Intl.Locale(observed.intlLocale).region||null;}catch(error){}"
        "try{observed.navigatorLanguageRegion=new Intl.Locale(observed.language).region||null;}catch(error){}"
        "try{const intlLocale=new Intl.Locale(observed.intlLocale);"
        "observed.localeWeekInfo=intlLocale.getWeekInfo?intlLocale.getWeekInfo():null;"
        "observed.localeTextInfo=intlLocale.getTextInfo?intlLocale.getTextInfo():null;"
        "observed.localeHourCycles=intlLocale.getHourCycles?intlLocale.getHourCycles():null;"
        "observed.localeCalendars=intlLocale.getCalendars?intlLocale.getCalendars():null;"
        "observed.localeNumberingSystems=intlLocale.getNumberingSystems?intlLocale.getNumberingSystems():null;"
        "observed.localeTimeZones=intlLocale.getTimeZones?intlLocale.getTimeZones():null;"
        "}catch(error){observed.localeInfoError=String(error&&error.name||error);}"
        "try{observed.regionDisplayName=new Intl.DisplayNames(undefined,{type:'region'}).of('US');}catch(error){}"
        "try{const fixedDate=new Date(Date.UTC(2026,4,22,12,0,0));"
        "const timeZoneFormatter=new Intl.DateTimeFormat(undefined,{weekday:'long',year:'numeric',month:'long',"
        "day:'numeric',hour:'numeric',minute:'numeric',timeZoneName:'long'});"
        "observed.timeZoneLongFormat=timeZoneFormatter.format(fixedDate);"
        "const timeZonePart=timeZoneFormatter.formatToParts(fixedDate).find((part)=>part.type==='timeZoneName');"
        "observed.timeZoneNameLong=timeZonePart?timeZonePart.value:null;"
        "}catch(error){observed.timeZoneNameError=String(error&&error.name||error);}"
        "try{"
        "if(navigator.userAgentData&&navigator.userAgentData.getHighEntropyValues){"
        "observed.userAgentDataHighEntropy=await navigator.userAgentData.getHighEntropyValues(["
        "'architecture','bitness','formFactor','fullVersionList','model','platform',"
        "'platformVersion','uaFullVersion','wow64','mobile'"
        "]);"
        "}"
        "}catch(error){observed.userAgentDataHighEntropyError=String(error&&error.name||error);}"
        "try{"
        "if(navigator.permissions&&navigator.permissions.query){"
        "observed.geolocationPermission=(await navigator.permissions.query({name:'geolocation'})).state;"
        "}"
        "}catch(error){"
        "observed.geolocationPermissionError=String(error&&error.name||error);"
        "}"
        "try{const connection=navigator.connection||navigator.mozConnection||navigator.webkitConnection;"
        "if(connection){"
        "observed.connectionEffectiveType=connection.effectiveType||null;"
        "observed.connectionSaveData=Boolean(connection.saveData);"
        "observed.connectionType=connection.type||null;"
        "}}catch(error){observed.connectionError=String(error&&error.name||error);}"
        "try{"
        "if(observed.webrtcSupported){"
        "const candidates=[];"
        "const pc=new RTCPeerConnection({iceServers:[]});"
        "pc.createDataChannel('agent-manager-us-probe');"
        "pc.onicecandidate=(event)=>{if(event.candidate&&event.candidate.candidate){candidates.push(event.candidate.candidate);}};"
        "await pc.setLocalDescription(await pc.createOffer());"
        "await new Promise((resolve)=>{"
        "let resolved=false;"
        "const finish=()=>{if(!resolved){resolved=true;resolve();}};"
        "pc.onicegatheringstatechange=()=>{if(pc.iceGatheringState==='complete'){finish();}};"
        "setTimeout(finish,1500);"
        "});"
        "pc.close();"
        "const privatePattern=/\\b(10(?:\\.\\d{1,3}){3}|192\\.168(?:\\.\\d{1,3}){2}|172\\.(?:1[6-9]|2\\d|3[0-1])(?:\\.\\d{1,3}){2}|127(?:\\.\\d{1,3}){3}|169\\.254(?:\\.\\d{1,3}){2}|::1|f[cd][0-9a-f]{2}:|fe80:)\\b/i;"
        "observed.webrtcCandidateCount=candidates.length;"
        "observed.webrtcHostCandidateCount=candidates.filter((candidate)=>/\\btyp host\\b/i.test(candidate)).length;"
        "observed.webrtcMdnsCandidateCount=candidates.filter((candidate)=>/\\.local\\b/i.test(candidate)).length;"
        "observed.webrtcPrivateIpCandidateCount=candidates.filter((candidate)=>privatePattern.test(candidate)).length;"
        "observed.webrtcLocalIpLeak=observed.webrtcHostCandidateCount>0||observed.webrtcMdnsCandidateCount>0||observed.webrtcPrivateIpCandidateCount>0;"
        "}"
        "}catch(error){"
        "observed.webrtcProbeError=String(error&&error.name||error);"
        "}"
        "document.body.textContent="
        + marker
        + "+JSON.stringify(observed);"
        "})();"
        "</script></html>"
    )


class RuntimeProbeHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        self.server.observed_headers["accept-language"] = self.headers.get("Accept-Language")
        self.server.observed_headers["user-agent"] = self.headers.get("User-Agent")
        self.server.observed_headers["sec-ch-ua"] = self.headers.get("Sec-CH-UA")
        self.server.observed_headers["sec-ch-ua-platform"] = self.headers.get("Sec-CH-UA-Platform")
        self.server.observed_headers["sec-ch-ua-mobile"] = self.headers.get("Sec-CH-UA-Mobile")
        body = runtime_probe_html().encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args) -> None:
        return


def start_runtime_probe_server() -> tuple[http.server.ThreadingHTTPServer, threading.Thread, str]:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), RuntimeProbeHandler)
    server.observed_headers = {}
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f"http://127.0.0.1:{server.server_port}/runtime-us-probe"
    return server, thread, url


def runtime_context_from_output(output: str) -> dict | None:
    marker_index = output.rfind(RUNTIME_JSON_MARKER)
    if marker_index == -1:
        return None
    json_start = marker_index + len(RUNTIME_JSON_MARKER)
    json_end = output.find("</", json_start)
    payload_text = output[json_start:] if json_end == -1 else output[json_start:json_end]
    try:
        payload = json.loads(payload_text)
    except json.JSONDecodeError:
        return None
    return payload if isinstance(payload, dict) else None


def chrome_major_version(user_agent: str) -> str | None:
    match = re.search(r"\bChrome/(\d+)\.", user_agent)
    return match.group(1) if match else None


def user_agent_data_brands_match_chrome_major(brands, user_agent: str) -> bool:
    major_version = chrome_major_version(user_agent)
    if major_version is None or not isinstance(brands, list):
        return False
    for brand in brands:
        if not isinstance(brand, dict):
            continue
        if brand.get("brand") in {"Chromium", "Google Chrome"} and str(brand.get("version")) == major_version:
            return True
    return False


def sec_ch_ua_matches_chrome_major(value, user_agent: str) -> bool:
    text = clean_string(value)
    major_version = chrome_major_version(user_agent)
    if text is None or major_version is None:
        return False
    return f'v="{major_version}"' in text and ('"Chromium"' in text or '"Google Chrome"' in text)


def positive_integer(value) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def expected_pdf_plugin_entries() -> list[dict]:
    mime_types = [
        {
            "description": US_DESKTOP_PDF_DESCRIPTION,
            "suffixes": US_DESKTOP_PDF_SUFFIXES,
            "type": mime_type,
        }
        for mime_type in US_DESKTOP_PDF_MIME_TYPES
    ]
    return [
        {
            "description": US_DESKTOP_PDF_DESCRIPTION,
            "filename": US_DESKTOP_PDF_PLUGIN_FILENAME,
            "length": len(US_DESKTOP_PDF_MIME_TYPES),
            "name": plugin_name,
            "types": [dict(mime_type) for mime_type in mime_types],
        }
        for plugin_name in US_DESKTOP_PDF_PLUGIN_NAMES
    ]


def expected_pdf_mime_type_entries() -> list[dict]:
    return [
        {
            "description": US_DESKTOP_PDF_DESCRIPTION,
            "enabledPlugin": US_DESKTOP_PDF_PLUGIN_NAMES[0],
            "suffixes": US_DESKTOP_PDF_SUFFIXES,
            "type": mime_type,
        }
        for mime_type in US_DESKTOP_PDF_MIME_TYPES
    ]


def pdf_plugin_surface_matches(observed: dict) -> bool:
    return observed.get("plugins") == expected_pdf_plugin_entries() and observed.get("mimeTypes") == expected_pdf_mime_type_entries()


def user_agent_high_entropy_matches_desktop_surface(
    high_entropy,
    user_agent: str,
    expected_client_hints_platform: str,
) -> bool:
    if not isinstance(high_entropy, dict) or not isinstance(user_agent, str):
        return False
    if high_entropy.get("platform") != expected_client_hints_platform:
        return False
    if high_entropy.get("mobile") is not False:
        return False
    if high_entropy.get("model") not in ("", None):
        return False
    if high_entropy.get("wow64") not in (False, None):
        return False

    full_version_list = high_entropy.get("fullVersionList")
    if isinstance(full_version_list, list) and full_version_list:
        if not user_agent_data_brands_match_chrome_major(full_version_list, user_agent):
            return False

    ua_full_version = clean_string(high_entropy.get("uaFullVersion"))
    major_version = chrome_major_version(user_agent)
    if ua_full_version and (major_version is None or not ua_full_version.startswith(f"{major_version}.")):
        return False

    if "Macintosh; Intel Mac OS X 10_15_7" in user_agent:
        if high_entropy.get("platformVersion") not in ("", "10_15_7", None):
            return False
        if high_entropy.get("architecture") not in ("", "x86", None):
            return False

    return True


def runtime_desktop_surface_mismatches(runtime_context: dict) -> tuple[str, ...]:
    mismatches = []
    if runtime_context.get("innerWidth") != US_DESKTOP_VIEWPORT["width"]:
        mismatches.append("runtime viewport width")
    if runtime_context.get("innerHeight") != US_DESKTOP_VIEWPORT["height"]:
        mismatches.append("runtime viewport height")
    if runtime_context.get("screenWidth") != US_DESKTOP_SCREEN["width"]:
        mismatches.append("runtime screen width")
    if runtime_context.get("screenHeight") != US_DESKTOP_SCREEN["height"]:
        mismatches.append("runtime screen height")
    if runtime_context.get("availWidth") != US_DESKTOP_SCREEN["width"]:
        mismatches.append("runtime screen available width")
    if runtime_context.get("availHeight") != US_DESKTOP_SCREEN["height"]:
        mismatches.append("runtime screen available height")
    if runtime_context.get("screenColorDepth") != US_DESKTOP_SCREEN_COLOR_DEPTH:
        mismatches.append("runtime screen color depth")
    if runtime_context.get("screenPixelDepth") != US_DESKTOP_SCREEN_COLOR_DEPTH:
        mismatches.append("runtime screen pixel depth")
    if runtime_context.get("screenOrientationType") != US_DESKTOP_SCREEN_ORIENTATION_TYPE:
        mismatches.append("runtime screen orientation type")
    if runtime_context.get("screenOrientationAngle") != US_DESKTOP_SCREEN_ORIENTATION_ANGLE:
        mismatches.append("runtime screen orientation angle")
    if runtime_context.get("devicePixelRatio") != US_DESKTOP_DEVICE_SCALE_FACTOR:
        mismatches.append("runtime device scale factor")
    if runtime_context.get("maxTouchPoints") != 0:
        mismatches.append("runtime touch points")
    if runtime_context.get("coarsePointer") is not False:
        mismatches.append("runtime coarse pointer")
    if runtime_context.get("finePointer") is not True:
        mismatches.append("runtime fine pointer")
    if runtime_context.get("hoverHover") is not True:
        mismatches.append("runtime hover")
    if runtime_context.get("lightColorScheme") is not True or runtime_context.get("darkColorScheme") is not False:
        mismatches.append("runtime color scheme")
    if runtime_context.get("pdfViewerEnabled") is not True:
        mismatches.append("runtime pdf viewer")
    if not positive_integer(runtime_context.get("pluginsLength")):
        mismatches.append("runtime plugins")
    if not positive_integer(runtime_context.get("mimeTypesLength")):
        mismatches.append("runtime mime types")
    if not pdf_plugin_surface_matches(runtime_context):
        mismatches.append("runtime PDF plugin surface")
    if runtime_context.get("speechSynthesisPresent") is not False:
        mismatches.append("runtime speech synthesis")
    if runtime_context.get("localFontAccessPresent") is not False:
        mismatches.append("runtime local font access")
    if runtime_context.get("bluetoothPresent") is not False:
        mismatches.append("runtime Bluetooth API")
    if runtime_context.get("usbPresent") is not False:
        mismatches.append("runtime USB API")
    if runtime_context.get("hidPresent") is not False:
        mismatches.append("runtime HID API")
    if runtime_context.get("xrPresent") is not False:
        mismatches.append("runtime WebXR API")
    if runtime_context.get("gamepadCount") != 0:
        mismatches.append("runtime gamepad count")
    return tuple(mismatches)


def validate_runtime_context(
    runtime_context: dict,
    timezone_id: str,
    user_agent: str,
    expected_client_hints_platform: str,
) -> tuple[str, ...]:
    mismatches = []
    if runtime_context.get("userAgent") != user_agent:
        mismatches.append("navigator.userAgent")
    if runtime_context.get("userAgentHeader") != user_agent:
        mismatches.append("User-Agent header")
    expected_navigator_platform = navigator_platform_from_user_agent(user_agent)
    if runtime_context.get("platform") != expected_navigator_platform:
        mismatches.append("navigator.platform")
    if runtime_context.get("vendor") != "Google Inc.":
        mismatches.append("navigator.vendor")
    if runtime_context.get("product") != "Gecko":
        mismatches.append("navigator.product")
    if runtime_context.get("productSub") != "20030107":
        mismatches.append("navigator.productSub")
    if runtime_context.get("appCodeName") != "Mozilla":
        mismatches.append("navigator.appCodeName")
    if runtime_context.get("appName") != "Netscape":
        mismatches.append("navigator.appName")
    if runtime_context.get("appVersion") != navigator_app_version_from_user_agent(user_agent):
        mismatches.append("navigator.appVersion")
    if runtime_context.get("userAgentDataMobile") not in (False, None):
        mismatches.append("runtime user agent mobile")
    if runtime_context.get("webdriver") not in (False, None):
        mismatches.append("runtime webdriver")
    if not user_agent_data_brands_match_chrome_major(runtime_context.get("userAgentDataBrands"), user_agent):
        mismatches.append("runtime user agent brands")
    user_agent_data_platform = runtime_context.get("userAgentDataPlatform")
    if user_agent_data_platform is not None and user_agent_data_platform != expected_client_hints_platform:
        mismatches.append("runtime user agent platform")
    if runtime_context.get("userAgentDataHighEntropyError") or not user_agent_high_entropy_matches_desktop_surface(
        runtime_context.get("userAgentDataHighEntropy"),
        user_agent,
        expected_client_hints_platform,
    ):
        mismatches.append("runtime user agent high entropy")
    if not sec_ch_ua_matches_chrome_major(runtime_context.get("secChUa"), user_agent):
        mismatches.append("Sec-CH-UA")
    sec_ch_ua_platform = runtime_context.get("secChUaPlatform")
    if sec_ch_ua_platform is not None and sec_ch_ua_platform.strip('"') != expected_client_hints_platform:
        mismatches.append("Sec-CH-UA-Platform")
    sec_ch_ua_mobile = runtime_context.get("secChUaMobile")
    if sec_ch_ua_mobile is not None and sec_ch_ua_mobile != "?0":
        mismatches.append("Sec-CH-UA-Mobile")
    if runtime_context.get("language") != APP_LOCALE:
        mismatches.append("navigator.language")
    if runtime_context.get("languages") != ["en-US", "en"]:
        mismatches.append("navigator.languages")
    mismatches.extend(runtime_desktop_surface_mismatches(runtime_context))
    if runtime_context.get("timezone") != timezone_id:
        mismatches.append("Intl timezone")
    expected_offset = expected_timezone_offset_minutes(timezone_id)
    if expected_offset is None:
        mismatches.append("timezone offset rule")
    elif runtime_context.get("timezoneOffset") != expected_offset:
        mismatches.append("timezone offset")
    if runtime_context.get("timeZoneNameError") or not timezone_display_matches(runtime_context, timezone_id):
        mismatches.append("Date timezone display")
    if runtime_context.get("intlLocale") != APP_LOCALE:
        mismatches.append("Intl locale")
    if runtime_context.get("intlCalendar") != "gregory":
        mismatches.append("Intl calendar")
    if runtime_context.get("intlNumberingSystem") != "latn":
        mismatches.append("Intl numbering system")
    if runtime_context.get("intlHourCycle") != "h12":
        mismatches.append("Intl hour cycle")
    if runtime_context.get("collatorLocale") != APP_LOCALE:
        mismatches.append("Intl collator locale")
    if runtime_context.get("pluralRulesLocale") != APP_LOCALE:
        mismatches.append("Intl plural rules locale")
    if runtime_context.get("relativeTimeFormatLocale") != APP_LOCALE:
        mismatches.append("Intl relative time format locale")
    if runtime_context.get("intlLocaleRegion") != "US":
        mismatches.append("Intl locale region")
    if runtime_context.get("navigatorLanguageRegion") != "US":
        mismatches.append("navigator language region")
    if runtime_context.get("localeInfoError"):
        mismatches.append("Intl locale info")
    if runtime_context.get("localeWeekInfo") != {"firstDay": 7, "weekend": [6, 7]}:
        mismatches.append("Intl locale week info")
    if runtime_context.get("localeTextInfo") != {"direction": "ltr"}:
        mismatches.append("Intl locale text info")
    locale_hour_cycles = runtime_context.get("localeHourCycles")
    if not isinstance(locale_hour_cycles, list) or "h12" not in locale_hour_cycles:
        mismatches.append("Intl locale hour cycles")
    locale_calendars = runtime_context.get("localeCalendars")
    if not isinstance(locale_calendars, list) or "gregory" not in locale_calendars:
        mismatches.append("Intl locale calendars")
    locale_numbering_systems = runtime_context.get("localeNumberingSystems")
    if not isinstance(locale_numbering_systems, list) or "latn" not in locale_numbering_systems:
        mismatches.append("Intl locale numbering systems")
    locale_time_zones = runtime_context.get("localeTimeZones")
    if not isinstance(locale_time_zones, list) or timezone_id not in locale_time_zones:
        mismatches.append("Intl locale time zones")
    if runtime_context.get("numberFormat") != "1,234,567.89":
        mismatches.append("Intl number format")
    if runtime_context.get("currencyFormat") != "$1,234.56":
        mismatches.append("Intl currency format")
    if runtime_context.get("dateFormat") != "5/22/26":
        mismatches.append("Intl date format")
    if runtime_context.get("regionDisplayName") != "United States":
        mismatches.append("Intl region display name")
    if runtime_context.get("listFormat") != "A, B, and C":
        mismatches.append("Intl list format")
    if not accept_language_is_us(runtime_context.get("acceptLanguageHeader")):
        mismatches.append("Accept-Language")
    if runtime_context.get("geolocationPermission") != "denied":
        mismatches.append("geolocation permission")
    if runtime_context.get("notificationPermission") != "denied":
        mismatches.append("Notification permission")
    if runtime_context.get("connectionError"):
        mismatches.append("network information probe")
    if runtime_context.get("connectionPresent") is True:
        if runtime_context.get("connectionEffectiveType") != US_DESKTOP_NETWORK_INFORMATION["effective_type"]:
            mismatches.append("network information effective type")
        if runtime_context.get("connectionSaveData") is not US_DESKTOP_NETWORK_INFORMATION["save_data"]:
            mismatches.append("network information save data")
    if runtime_context.get("profileWebrtcPolicy") != "disable_non_proxied_udp":
        mismatches.append("profile WebRTC policy")
    if runtime_context.get("profileWebrtcNonproxiedUdpEnabled") is not False:
        mismatches.append("profile WebRTC nonproxied UDP")
    if runtime_context.get("webrtcSupported") is not True:
        mismatches.append("WebRTC runtime probe support")
    if runtime_context.get("webrtcProbeError"):
        mismatches.append("WebRTC runtime probe")
    if runtime_context.get("webrtcLocalIpLeak") is not False:
        mismatches.append("WebRTC local IP leak")
    if runtime_context.get("webrtcPrivateIpCandidateCount") != 0:
        mismatches.append("WebRTC private IP candidates")
    if runtime_context.get("profileGeolocationSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile geolocation setting")
    if runtime_context.get("profileCameraSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile camera setting")
    if runtime_context.get("profileMicrophoneSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile microphone setting")
    if runtime_context.get("profileNotificationsSetting") != BLOCK_CONTENT_SETTING:
        mismatches.append("profile notifications setting")
    return tuple(mismatches)


def runtime_profile_policy_snapshot(user_data_dir: Path) -> dict:
    try:
        preferences = load_json_mapping(user_data_dir / "Default" / "Preferences")
    except ValueError:
        preferences = {}
    return {
        "profileWebrtcPolicy": nested_value(preferences, ("webrtc", "ip_handling_policy")),
        "profileWebrtcNonproxiedUdpEnabled": nested_value(preferences, ("webrtc", "nonproxied_udp_enabled")),
        "profileGeolocationSetting": nested_value(preferences, ("profile", "default_content_setting_values", "geolocation")),
        "profileCameraSetting": nested_value(
            preferences,
            ("profile", "default_content_setting_values", "media_stream_camera"),
        ),
        "profileMicrophoneSetting": nested_value(
            preferences,
            ("profile", "default_content_setting_values", "media_stream_mic"),
        ),
        "profileNotificationsSetting": nested_value(
            preferences,
            ("profile", "default_content_setting_values", "notifications"),
        ),
    }


def terminate_process_group(process: subprocess.Popen) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            return


def text_from_timeout_output(value) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="ignore")
    return str(value)


def collect_runtime_stdout(process: subprocess.Popen, timeout: float) -> tuple[str, str]:
    if process.stdout is None:
        return "", "stdout_unavailable"

    if os.name == "nt":
        try:
            stdout, _stderr = process.communicate(timeout=timeout)
            return text_from_timeout_output(stdout), f"exited_{process.returncode}"
        except subprocess.TimeoutExpired as exc:
            partial_stdout = text_from_timeout_output(exc.output)
            return partial_stdout, "timed_out"

    deadline = time.monotonic() + timeout
    chunks: list[bytes] = []
    stdout_fd = process.stdout.fileno()

    def decoded_output() -> str:
        return b"".join(chunks).decode("utf-8", errors="ignore")

    while True:
        output = decoded_output()
        if runtime_context_from_output(output) is not None:
            return output, "output_observed"

        if process.poll() is not None:
            while True:
                try:
                    chunk = os.read(stdout_fd, 4096)
                except BlockingIOError:
                    break
                if not chunk:
                    break
                chunks.append(chunk)
            return decoded_output(), f"exited_{process.returncode}"

        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return output, "timed_out"

        readable, _writable, _exceptional = select.select([stdout_fd], [], [], min(0.25, remaining))
        if not readable:
            continue
        chunk = os.read(stdout_fd, 4096)
        if chunk:
            chunks.append(chunk)


def verify_runtime_browser_context(
    launcher_path: Path,
    timezone_id: str,
    user_agent: str,
    expected_client_hints_platform: str,
    timeout: float,
) -> RuntimeResolution:
    if timeout <= 0:
        return RuntimeResolution(5, "invalid_timeout", errors=("runtime timeout must be positive",))
    if not launcher_path.exists() or not os.access(launcher_path, os.X_OK):
        return RuntimeResolution(5, "launcher_missing", errors=(str(launcher_path),))

    with tempfile.TemporaryDirectory(
        prefix="agent-manager-browser-us-runtime-",
        ignore_cleanup_errors=True,
    ) as temp_user_data_dir:
        temp_user_data_path = Path(temp_user_data_dir)
        sync_managed_browser_us_profile(temp_user_data_path)
        profile_policy = runtime_profile_policy_snapshot(temp_user_data_path)
        server, thread, runtime_url = start_runtime_probe_server()
        command = [
            str(launcher_path),
            "--headless",
            "--disable-gpu",
            "--disable-background-networking",
            # 探针是无头模式，--window-size 即视口（且本版 Chrome headless 会再扣 87px
            # 伪窗口高度）；launcher 里的 1365x907 是有头模式为抵消窗口装饰用的，
            # 这里以后出现的同名 flag 覆盖为无头探针所需尺寸。
            f"--window-size={US_DESKTOP_HEADLESS_PROBE_WINDOW['width']},{US_DESKTOP_HEADLESS_PROBE_WINDOW['height']}",
            "--virtual-time-budget=5000",
            "--dump-dom",
            runtime_url,
        ]
        env = os.environ.copy()
        env["AGENT_MANAGER_BROWSER_US_USER_DATA_DIR"] = temp_user_data_dir
        try:
            process = subprocess.Popen(
                command,
                cwd=str(launcher_path.parent),
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            stdout, process_state = collect_runtime_stdout(process, timeout)
            if process.poll() is None:
                terminate_process_group(process)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)

    runtime_context = runtime_context_from_output(stdout or "")
    if runtime_context is None:
        if process_state == "timed_out":
            process_state = "timed_out_without_output"
        return RuntimeResolution(
            5,
            "runtime_output_missing",
            process_state=process_state,
            errors=(f"runtime output marker missing; process_state={process_state}",),
        )
    runtime_context["acceptLanguageHeader"] = server.observed_headers.get("accept-language")
    runtime_context["userAgentHeader"] = server.observed_headers.get("user-agent")
    runtime_context["secChUa"] = server.observed_headers.get("sec-ch-ua")
    runtime_context["secChUaPlatform"] = server.observed_headers.get("sec-ch-ua-platform")
    runtime_context["secChUaMobile"] = server.observed_headers.get("sec-ch-ua-mobile")
    runtime_context.update(profile_policy)

    mismatches = validate_runtime_context(runtime_context, timezone_id, user_agent, expected_client_hints_platform)
    if mismatches:
        return RuntimeResolution(
            5,
            "runtime_mismatch_failed",
            observed=runtime_context,
            process_state=process_state,
            errors=mismatches,
        )
    return RuntimeResolution(0, "verified_runtime_us", observed=runtime_context, process_state="verified")


def coordinates_match(actual, expected, tolerance: float = 0.000001) -> bool:
    actual_number = parse_float(actual)
    expected_number = parse_float(expected)
    if actual_number is None or expected_number is None:
        return False
    return abs(actual_number - expected_number) <= tolerance

