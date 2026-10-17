/* Disposable native CI fault: keep the vendor setter, suppress only its reply. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <string.h>
#include <systemd/sd-bus.h>

int sd_bus_reply_method_return(sd_bus_message *call, const char *types, ...) {
    const char *member = sd_bus_message_get_member(call);
    if (member && strcmp(member, "SetRuntimeNTPServers") == 0)
        /* Consume the call; zero would let sd-bus emit UnknownMethod. */
        return 1;
    int (*reply)(sd_bus_message *, const char *, va_list) =
        dlsym(RTLD_NEXT, "sd_bus_reply_method_returnv");
    if (!reply)
        return -1;
    va_list arguments;
    va_start(arguments, types);
    int result = reply(call, types, arguments);
    va_end(arguments);
    return result;
}
