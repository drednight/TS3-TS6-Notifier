/*
 * TS3-TS6 Notifier — TeamSpeak 3 client plugin (tsn bridge).
 *
 * Streams client events (join / leave / move / talk / chat / poke / group
 * changes + full snapshots) to the local TS3-TS6 Notifier app over a
 * 127.0.0.1 TCP socket, one JSON object per line. This is the same approach
 * the Overwolf TeamSpeak plugin uses: events come from inside the client,
 * so there is no query connection to drop and no polling lag.
 *
 * Protocol (newline-delimited JSON, plugin -> app):
 *   {"e":"hello","v":"..."}                                   on connect
 *   {"e":"snapshot","server":"...","own":N,"owncid":N,
 *    "clients":[{"id":N,"name":"..","cid":N,"groups":".."}]}   full state
 *   {"e":"join"|"leave"|"move"|"talk"|"client", ...}          deltas
 *   talk: {"clid":N,"s":0|1,"w":0|1}   w = isReceivedWhisper (v1.4.12)
 *   {"e":"chat","kind":"channel|private|server","name":"..","text":".."}
 *   {"e":"poke","name":"..","text":".."}
 *   {"e":"disconnect"}
 * app -> plugin:
 *   {"cmd":"snapshot"}   re-send the snapshot
 *   {"cmd":"ping"}       answer {"e":"pong"}
 *
 * Build (x64, from a VS dev prompt):  build.bat
 */
#define _WIN32_WINNT 0x0601
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "teamspeak/public_errors.h"
#include "teamspeak/public_rare_definitions.h"
#include "ts3_functions.h"

#define TSN_PORT 48912
#define TSN_SEND_BUF (256 * 1024)

static struct TS3Functions ts3funcs;
static SOCKET tsn_sock = INVALID_SOCKET;
static CRITICAL_SECTION tsn_lock;
static volatile LONG tsn_running = 0;
static HANDLE tsn_thread = NULL;

/* ---------- tiny JSON helpers ---------- */

static void json_escape(const char* in, char* out, size_t cap) {
    size_t o = 0;
    for (size_t i = 0; in && in[i] && o + 8 < cap; i++) {
        unsigned char c = (unsigned char)in[i];
        if (c == '"' || c == '\\') { out[o++] = '\\'; out[o++] = (char)c; }
        else if (c == '\n') { out[o++] = '\\'; out[o++] = 'n'; }
        else if (c == '\r') { out[o++] = '\\'; out[o++] = 'r'; }
        else if (c == '\t') { out[o++] = '\\'; out[o++] = 't'; }
        else if (c < 0x20) { o += (size_t)sprintf(out + o, "\\u%04x", c); }
        else out[o++] = (char)c;
    }
    out[o] = 0;
}

static int tsn_try_connect(void) {
    if (tsn_sock != INVALID_SOCKET) return 1;
    SOCKET s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (s == INVALID_SOCKET) return 0;
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(TSN_PORT);
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    u_long nb = 1;
    ioctlsocket(s, FIONBIO, &nb);
    connect(s, (struct sockaddr*)&addr, sizeof(addr));
    fd_set wf; FD_ZERO(&wf); FD_SET(s, &wf);
    struct timeval tv; tv.tv_sec = 0; tv.tv_usec = 250000;
    if (select(0, NULL, &wf, NULL, &tv) <= 0) { closesocket(s); return 0; }
    int err = 0; int elen = sizeof(err);
    getsockopt(s, SOL_SOCKET, SO_ERROR, (char*)&err, &elen);
    if (err != 0) { closesocket(s); return 0; }
    nb = 0;
    ioctlsocket(s, FIONBIO, &nb);
    DWORD timeout = 2000;
    setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, (const char*)&timeout, sizeof(timeout));
    tsn_sock = s;
    return 1;
}

static int tsn_send_all(SOCKET s, const char* buf, int len) {
    int sent = 0;
    while (sent < len) {
        int n = send(s, buf + sent, len - sent, 0);
        if (n <= 0) return 0;
        sent += n;
    }
    return 1;
}

static void tsn_send_raw(const char* line, int len) {
    EnterCriticalSection(&tsn_lock);
    if (!tsn_try_connect()) { LeaveCriticalSection(&tsn_lock); return; }
    if (!tsn_send_all(tsn_sock, line, len)) {
        closesocket(tsn_sock);
        tsn_sock = INVALID_SOCKET;
        if (tsn_try_connect()) tsn_send_all(tsn_sock, line, len);
    }
    LeaveCriticalSection(&tsn_lock);
}

static void tsn_sendf(const char* fmt, ...) {
    static __declspec(thread) char buf[8192];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof(buf) - 2, fmt, ap);
    va_end(ap);
    if (n <= 0) return;
    if (n > (int)sizeof(buf) - 2) n = (int)sizeof(buf) - 2;
    buf[n++] = '\n';
    tsn_send_raw(buf, n);
}

/* ---------- state readers ---------- */

static char* str_dup_ts(uint64 schid, anyID clid, size_t flag) {
    char* s = NULL;
    char out[512];
    out[0] = 0;
    if (ts3funcs.getClientVariableAsString(schid, clid, flag, &s) == ERROR_ok && s) {
        strncpy(out, s, sizeof(out) - 1);
        out[sizeof(out) - 1] = 0;
        ts3funcs.freeMemory(s);
    }
    char* dup = (char*)malloc(strlen(out) + 1);
    if (dup) strcpy(dup, out);
    return dup;
}

static void chan_name(uint64 schid, uint64 cid, char* out, size_t cap) {
    out[0] = 0;
    char* s = NULL;
    if (cid > 0 && ts3funcs.getChannelVariableAsString(schid, cid, CHANNEL_NAME, &s) == ERROR_ok && s) {
        strncpy(out, s, cap - 1);
        out[cap - 1] = 0;
        ts3funcs.freeMemory(s);
    }
}

static void send_client_info(uint64 schid, anyID clid, const char* ev, uint64 cid_from, uint64 cid_to, const char* reason) {
    char* name = str_dup_ts(schid, clid, CLIENT_NICKNAME);
    char* groups = str_dup_ts(schid, clid, CLIENT_SERVERGROUPS);
    char en[300], eg[300], ec[300];
    char cname[256];
    chan_name(schid, strcmp(ev, "leave") == 0 ? cid_from : cid_to, cname, sizeof(cname));
    json_escape(name ? name : "", en, sizeof(en));
    json_escape(groups ? groups : "", eg, sizeof(eg));
    json_escape(cname, ec, sizeof(ec));
    if (strcmp(ev, "move") == 0) {
        tsn_sendf("{\"e\":\"move\",\"clid\":%u,\"from\":%llu,\"to\":%llu,\"name\":\"%s\",\"groups\":\"%s\",\"cname\":\"%s\"}",
                  (unsigned)clid, (unsigned long long)cid_from, (unsigned long long)cid_to, en, eg, ec);
    } else if (strcmp(ev, "leave") == 0) {
        tsn_sendf("{\"e\":\"leave\",\"clid\":%u,\"cid\":%llu,\"name\":\"%s\",\"groups\":\"%s\",\"reason\":\"%s\"}",
                  (unsigned)clid, (unsigned long long)cid_from, en, eg, reason ? reason : "");
    } else if (strcmp(ev, "client") == 0) {
        tsn_sendf("{\"e\":\"client\",\"clid\":%u,\"name\":\"%s\",\"groups\":\"%s\"}", (unsigned)clid, en, eg);
    } else {
        tsn_sendf("{\"e\":\"%s\",\"clid\":%u,\"cid\":%llu,\"name\":\"%s\",\"groups\":\"%s\",\"cname\":\"%s\"}",
                  ev, (unsigned)clid, (unsigned long long)cid_to, en, eg, ec);
    }
    free(name);
    free(groups);
}

static void send_snapshot(uint64 schid) {
    static __declspec(thread) char buf[TSN_SEND_BUF];
    char* sname = NULL;
    char eserver[300];
    eserver[0] = 0;
    if (ts3funcs.getServerVariableAsString(schid, VIRTUALSERVER_NAME, &sname) == ERROR_ok && sname) {
        json_escape(sname, eserver, sizeof(eserver));
        ts3funcs.freeMemory(sname);
    }
    anyID own = 0;
    ts3funcs.getClientID(schid, &own);
    uint64 owncid = 0;
    ts3funcs.getChannelOfClient(schid, own, &owncid);

    char owncname[256], eowncname[300];
    chan_name(schid, owncid, owncname, sizeof(owncname));
    json_escape(owncname, eowncname, sizeof(eowncname));
    int n = snprintf(buf, sizeof(buf) - 2,
                     "{\"e\":\"snapshot\",\"server\":\"%s\",\"own\":%u,\"owncid\":%llu,\"owncname\":\"%s\",\"clients\":[",
                     eserver, (unsigned)own, (unsigned long long)owncid, eowncname);
    anyID* ids = NULL;
    if (ts3funcs.getClientList(schid, &ids) == ERROR_ok && ids) {
        for (int i = 0; ids[i] && n < (int)sizeof(buf) - 2048; i++) {
            char* name = str_dup_ts(schid, ids[i], CLIENT_NICKNAME);
            char* groups = str_dup_ts(schid, ids[i], CLIENT_SERVERGROUPS);
            uint64 cid = 0;
            ts3funcs.getChannelOfClient(schid, ids[i], &cid);
            char en[300], eg[300], ec[300];
            char cname[256];
            chan_name(schid, cid, cname, sizeof(cname));
            json_escape(name ? name : "", en, sizeof(en));
            json_escape(groups ? groups : "", eg, sizeof(eg));
            json_escape(cname, ec, sizeof(ec));
            n += snprintf(buf + n, sizeof(buf) - n - 2,
                          "%s{\"id\":%u,\"name\":\"%s\",\"cid\":%llu,\"groups\":\"%s\",\"cname\":\"%s\"}",
                          i ? "," : "", (unsigned)ids[i], en, (unsigned long long)cid, eg, ec);
            free(name);
            free(groups);
        }
        ts3funcs.freeMemory(ids);
    }
    if (n > (int)sizeof(buf) - 8) n = (int)sizeof(buf) - 8;
    n += snprintf(buf + n, sizeof(buf) - n - 2, "]}");
    buf[n++] = '\n';
    tsn_send_raw(buf, n);
}

static void send_snapshot_all(void) {
    uint64* schids = NULL;
    if (ts3funcs.getServerConnectionHandlerList(&schids) != ERROR_ok || !schids) return;
    for (int i = 0; schids[i]; i++) {
        anyID own = 0;
        if (ts3funcs.getClientID(schids[i], &own) == ERROR_ok && own > 0) send_snapshot(schids[i]);
    }
    ts3funcs.freeMemory(schids);
}

/* ---------- exported plugin interface ---------- */

#ifdef _WIN32
#define PLUGINS_EXPORTDLL __declspec(dllexport)
#else
#define PLUGINS_EXPORTDLL
#endif

PLUGINS_EXPORTDLL const char* ts3plugin_name(void) { return "TS3-TS6 Notifier bridge"; }
PLUGINS_EXPORTDLL const char* ts3plugin_version(void) { return "1.1.0"; }
/* TS3 client 3.6.x requires API 26 (error on load: "Api version is not
 * compatible: 23 (minimum: 26, current: 26)"). */
PLUGINS_EXPORTDLL int ts3plugin_apiVersion(void) { return 26; }
PLUGINS_EXPORTDLL const char* ts3plugin_author(void) { return "TS3-TS6 Notifier"; }
PLUGINS_EXPORTDLL const char* ts3plugin_description(void) {
    return "Streams TeamSpeak events to the local TS3-TS6 Notifier overlay (no ClientQuery polling).";
}
PLUGINS_EXPORTDLL void ts3plugin_setFunctionPointers(const struct TS3Functions funcs) { ts3funcs = funcs; }
PLUGINS_EXPORTDLL void ts3plugin_registerPluginID(const char* id) { (void)id; }
PLUGINS_EXPORTDLL int ts3plugin_offersConfigure(void) { return 0; }
PLUGINS_EXPORTDLL void ts3plugin_configure(void* handle, void* qParentWidget) { (void)handle; (void)qParentWidget; }
PLUGINS_EXPORTDLL int ts3plugin_requestAutoload(void) { return 1; }
PLUGINS_EXPORTDLL void ts3plugin_freeMemory(void* data) { if (data) free(data); }

static DWORD WINAPI tsn_reader(LPVOID arg);

static DWORD WINAPI tsn_worker(LPVOID arg) {
    (void)arg;
    int tick = 0;
    while (tsn_running) {
        EnterCriticalSection(&tsn_lock);
        int was = (tsn_sock != INVALID_SOCKET);
        int ok = tsn_try_connect();
        if (ok && !was) {
            char hello[64];
            int n = snprintf(hello, sizeof(hello), "{\"e\":\"hello\",\"v\":\"%s\"}\n", ts3plugin_version());
            if (!tsn_send_all(tsn_sock, hello, n)) {
                closesocket(tsn_sock);
                tsn_sock = INVALID_SOCKET;
                ok = 0;
            }
        }
        if (ok && was && (tick % 5) == 0) {
            /* Heartbeat: the app uses traffic recency to tell a live plugin
             * from a dead one — without it the overlay fell back to
             * ClientQuery every quiet minute and flickered. */
            if (!tsn_send_all(tsn_sock, "{\"e\":\"ping\"}\n", 13)) {
                closesocket(tsn_sock);
                tsn_sock = INVALID_SOCKET;
                ok = 0;
            }
        }
        LeaveCriticalSection(&tsn_lock);
        if (ok && !was) send_snapshot_all();
        tick++;
        Sleep(ok ? 1000 : 3000);
    }
    return 0;
}

PLUGINS_EXPORTDLL int ts3plugin_init(void) {
    InitializeCriticalSection(&tsn_lock);
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);
    tsn_running = 1;
    tsn_thread = CreateThread(NULL, 0, tsn_worker, NULL, 0, NULL);
    CreateThread(NULL, 0, tsn_reader, NULL, 0, NULL);
    return 0;
}

PLUGINS_EXPORTDLL void ts3plugin_shutdown(void) {
    tsn_running = 0;
    if (tsn_thread) { WaitForSingleObject(tsn_thread, 3000); CloseHandle(tsn_thread); }
    EnterCriticalSection(&tsn_lock);
    if (tsn_sock != INVALID_SOCKET) { closesocket(tsn_sock); tsn_sock = INVALID_SOCKET; }
    LeaveCriticalSection(&tsn_lock);
    WSACleanup();
    DeleteCriticalSection(&tsn_lock);
}

/* ---------- events ---------- */

PLUGINS_EXPORTDLL void ts3plugin_onConnectStatusChangeEvent(uint64 schid, int newStatus, unsigned int errorNumber) {
    (void)errorNumber;
    if (newStatus == STATUS_DISCONNECTED) {
        tsn_sendf("{\"e\":\"disconnect\",\"schid\":%llu}", (unsigned long long)schid);
    } else if (newStatus >= STATUS_CONNECTED) {
        send_snapshot(schid);
    }
}

PLUGINS_EXPORTDLL void ts3plugin_currentServerConnectionChanged(uint64 schid) {
    if (schid) send_snapshot(schid);
}

PLUGINS_EXPORTDLL void ts3plugin_onClientMoveEvent(uint64 schid, anyID clid, uint64 oldChannelID, uint64 newChannelID, int visibility, const char* moveMessage) {
    (void)visibility; (void)moveMessage;
    if (oldChannelID == 0 && newChannelID != 0) send_client_info(schid, clid, "join", 0, newChannelID, "");
    else if (newChannelID == 0 && oldChannelID != 0) send_client_info(schid, clid, "leave", oldChannelID, 0, "left");
    else if (oldChannelID != 0 && newChannelID != 0) send_client_info(schid, clid, "move", oldChannelID, newChannelID, "");
}

PLUGINS_EXPORTDLL void ts3plugin_onClientMoveSubscriptionEvent(uint64 schid, anyID clid, uint64 oldChannelID, uint64 newChannelID, int visibility) {
    (void)visibility;
    if (newChannelID != 0 && oldChannelID == 0) send_client_info(schid, clid, "join", 0, newChannelID, "");
    else if (newChannelID == 0 && oldChannelID != 0) send_client_info(schid, clid, "leave", oldChannelID, 0, "lost");
}

PLUGINS_EXPORTDLL void ts3plugin_onClientMoveTimeoutEvent(uint64 schid, anyID clid, uint64 oldChannelID, uint64 newChannelID, int visibility, const char* timeoutMessage) {
    (void)visibility; (void)timeoutMessage; (void)newChannelID;
    if (oldChannelID != 0) send_client_info(schid, clid, "leave", oldChannelID, 0, "timeout");
}

PLUGINS_EXPORTDLL void ts3plugin_onUpdateClientEvent(uint64 schid, anyID clid, anyID invokerID, const char* invokerName, const char* invokerUniqueIdentifier) {
    (void)invokerID; (void)invokerName; (void)invokerUniqueIdentifier;
    send_client_info(schid, clid, "client", 0, 0, "");
}

PLUGINS_EXPORTDLL void ts3plugin_onTalkStatusChangeEvent(uint64 schid, int status, int isReceivedWhisper, anyID clid) {
    /* «w» = isReceivedWhisper: the overlay shows whisperers even when they
     * speak from another channel (added in v1.4.12). */
    tsn_sendf("{\"e\":\"talk\",\"clid\":%u,\"s\":%d,\"w\":%d,\"schid\":%llu}", (unsigned)clid, status, isReceivedWhisper, (unsigned long long)schid);
}

PLUGINS_EXPORTDLL int ts3plugin_onTextMessageEvent(uint64 schid, anyID targetMode, anyID toID, anyID fromID, const char* fromName, const char* fromUniqueIdentifier, const char* message, int ffIgnored) {
    (void)schid; (void)toID; (void)fromID; (void)fromUniqueIdentifier; (void)ffIgnored;
    const char* kind = targetMode == 1 ? "private" : (targetMode == 3 ? "server" : "channel");
    char en[300], et[2048];
    json_escape(fromName ? fromName : "", en, sizeof(en));
    json_escape(message ? message : "", et, sizeof(et));
    tsn_sendf("{\"e\":\"chat\",\"kind\":\"%s\",\"clid\":%u,\"name\":\"%s\",\"text\":\"%s\"}", kind, (unsigned)fromID, en, et);
    return 0;
}

PLUGINS_EXPORTDLL int ts3plugin_onClientPokeEvent(uint64 schid, anyID fromClientID, const char* pokerName, const char* pokerUniqueIdentity, const char* message, int ffIgnored) {
    (void)schid; (void)fromClientID; (void)pokerUniqueIdentity; (void)ffIgnored;
    char en[300], et[1024];
    json_escape(pokerName ? pokerName : "", en, sizeof(en));
    json_escape(message ? message : "", et, sizeof(et));
    tsn_sendf("{\"e\":\"poke\",\"clid\":%u,\"name\":\"%s\",\"text\":\"%s\"}", (unsigned)fromClientID, en, et);
    return 0;
}

/* ---------- app -> plugin commands (snapshot refresh / keepalive) ---------- */

static DWORD WINAPI tsn_reader(LPVOID arg) {
    (void)arg;
    char buf[4096];
    size_t used = 0;
    while (tsn_running) {
        Sleep(50);
        /* Reading is optional (the app may never send), so poll cheaply. */
        EnterCriticalSection(&tsn_lock);
        SOCKET s = tsn_sock;
        LeaveCriticalSection(&tsn_lock);
        if (s == INVALID_SOCKET) { used = 0; continue; }
        u_long avail = 0;
        if (ioctlsocket(s, FIONREAD, &avail) != 0 || avail == 0) continue;
        int n = recv(s, buf + used, (int)(sizeof(buf) - used - 1), 0);
        if (n <= 0) {
            EnterCriticalSection(&tsn_lock);
            if (tsn_sock == s) { closesocket(s); tsn_sock = INVALID_SOCKET; }
            LeaveCriticalSection(&tsn_lock);
            used = 0;
            continue;
        }
        used += (size_t)n;
        buf[used] = 0;
        char* start = buf;
        char* nl;
        while ((nl = strchr(start, '\n')) != NULL) {
            *nl = 0;
            if (strstr(start, "\"snapshot\"")) send_snapshot_all();
            else if (strstr(start, "\"ping\"")) tsn_sendf("{\"e\":\"pong\"}");
            start = nl + 1;
        }
        memmove(buf, start, strlen(start) + 1);
        used = strlen(start);
    }
    return 0;
}

