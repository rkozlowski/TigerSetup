/*
 * The loader: the small executable every generated `Setup.exe` begins with.
 *
 * A generated installer is `[loader][compressed engine][payload][metadata]
 * [footer]` (`tigersetup-format`). The loader's whole job is to get the
 * real engine running against the file it came from:
 *
 * 1. locate and validate the footer of its own file;
 * 2. decompress the engine block into a fresh temporary file, checking
 *    its length and SHA-256 against the footer before anything is
 *    executed;
 * 3. start that engine with this process's own command line, verbatim,
 *    plus `--package <this file>`, so the engine reads the metadata and
 *    the payload from the original `Setup.exe` and relaunches *that* when
 *    it needs an elevated run or a temporary uninstaller copy;
 * 4. wait, propagate the engine's exit code, and remove the temporary.
 *
 * Nothing else lives here: no metadata, no payload, no state, no
 * transaction. The loader does not decide anything about the run — the
 * engine parses the command line and the engine asks for elevation — so a
 * machine-scope run is an elevated `Setup.exe` (this loader again) that
 * extracts its own engine under a system-owned root nobody else can write.
 *
 * Where the engine is extracted. Unelevated: `%TEMP%\TigerSetup\<pid>-
 * <tick>-<attempt>\<Setup.exe's own file name>`, which is the invoking
 * user's own folder. Elevated: a fresh directory under `%SystemRoot%\Temp`,
 * created with an access control list that grants SYSTEM and Administrators
 * alone, owner Administrators, atomically at creation — an executable about
 * to run elevated must never sit in a folder the invoking user can write. A
 * name that already exists is never adopted. The file keeps the package's
 * own name, so Task Manager, the Restart Manager and a crash dialog all say
 * which installer is running.
 *
 * Failure. A file that does not carry a valid engine — no footer, a hash
 * that does not match, a block that does not decompress — is reported as a
 * damaged package: on standard error, and in a message box when the loader
 * was started with no arguments at all (a double-click), and exits 2, the
 * engine's own code for an invalid package. Nothing half-extracted is left
 * behind: the temporary is removed on every path.
 *
 * The program is C and Win32 only. It links no C runtime service beyond
 * what the compiler itself emits (`memcpy`, `memset`, the `/GS` cookie):
 * memory comes from the process heap, hashing from CNG, and Zstandard
 * decompression from libzstd's decoder compiled into it, with its
 * allocator routed to the process heap as well (`zstd_alloc.h`).
 */

#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <bcrypt.h>
#include <sddl.h>
#include <zstd.h>
#include "zstd_alloc.h"

/* --------------------------------------------------------------------- */
/* Constants                                                              */
/* --------------------------------------------------------------------- */

/* The engine's exit code for an invalid package, which is what a file the
 * loader cannot bootstrap is. */
#define INVALID 2u

/* The fixed 320-byte trailer that maps the installer file (format 3). */
#define FOOTER_LEN 320u
#define FORMAT_MAJOR 3u
#define FORMAT_MINOR 0u
#define CRC_OFFSET 308u
#define MAGIC_TAIL_OFFSET 312u

/* The window a TigerSetup stream may use; a frame asking for more is
 * invalid rather than an allocation. */
#define MAX_WINDOW_LOG 27

/* Reads of the package go in this unit. */
#define READ_CHUNK (256u * 1024u)

/* The access control list of an elevated extraction directory: owner
 * Administrators, protected, full control for SYSTEM and Administrators
 * and nobody else — the same list the engine puts on a machine-scope
 * state directory. */
#define ELEVATED_DACL L"O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"

/* The argument the engine reads the package path from. */
#define PACKAGE_ARGUMENT L"--package"

/* A day, in the 100-nanosecond units of a FILETIME. */
#define STALE_AGE (24ull * 60ull * 60ull * 10000000ull)

static const BYTE MAGIC_HEAD[8] = {'T', 'I', 'G', 'E', 'R', 'S', 'T', 'P'};
static const BYTE MAGIC_TAIL[8] = {'P', 'T', 'S', 'R', 'E', 'G', 'I', 'T'};

/* --------------------------------------------------------------------- */
/* Memory and text                                                        */
/* --------------------------------------------------------------------- */

static void *allocate(SIZE_T bytes)
{
    return HeapAlloc(GetProcessHeap(), 0, bytes);
}

static void release(void *block)
{
    if (block != NULL) {
        HeapFree(GetProcessHeap(), 0, block);
    }
}

/* libzstd's allocator (`zstd_alloc.h`): the process heap. */
void *tigersetup_loader_zstd_malloc(size_t size)
{
    return allocate(size);
}

void *tigersetup_loader_zstd_calloc(size_t count, size_t size)
{
    if (size != 0 && count > ((size_t)-1) / size) {
        return NULL;
    }
    return HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, count * size);
}

void tigersetup_loader_zstd_free(void *block)
{
    release(block);
}

/* A growable UTF-16 string, always zero-terminated once it holds anything.
 * Memory is never expected to run out for the few kilobytes the loader
 * formats; if it does, the loader ends as an aborted process would, with
 * the invalid-package code. */
typedef struct Text {
    WCHAR *chars;
    SIZE_T length;
    SIZE_T capacity;
} Text;

static void text_reserve(Text *text, SIZE_T extra)
{
    SIZE_T needed = text->length + extra + 1;
    SIZE_T capacity;
    WCHAR *grown;
    if (needed <= text->capacity) {
        return;
    }
    capacity = text->capacity == 0 ? 256 : text->capacity;
    while (capacity < needed) {
        capacity *= 2;
    }
    grown = text->chars == NULL
        ? (WCHAR *)HeapAlloc(GetProcessHeap(), 0, capacity * sizeof(WCHAR))
        : (WCHAR *)HeapReAlloc(GetProcessHeap(), 0, text->chars, capacity * sizeof(WCHAR));
    if (grown == NULL) {
        ExitProcess(INVALID);
    }
    text->chars = grown;
    text->capacity = capacity;
}

static BOOL bytes_equal(const BYTE *a, const BYTE *b, SIZE_T count)
{
    SIZE_T i;
    BYTE difference = 0;
    for (i = 0; i < count; i++) {
        difference |= (BYTE)(a[i] ^ b[i]);
    }
    return difference == 0;
}

static void text_push(Text *text, const WCHAR *chars, SIZE_T count)
{
    text_reserve(text, count);
    memcpy(text->chars + text->length, chars, count * sizeof(WCHAR));
    text->length += count;
    text->chars[text->length] = 0;
}

static SIZE_T wide_length(const WCHAR *chars)
{
    SIZE_T length = 0;
    while (chars[length] != 0) {
        length++;
    }
    return length;
}

static void text_push_wide(Text *text, const WCHAR *chars)
{
    text_push(text, chars, wide_length(chars));
}

static void text_push_ascii(Text *text, const char *chars)
{
    while (*chars != 0) {
        WCHAR wide = (WCHAR)(unsigned char)*chars++;
        text_push(text, &wide, 1);
    }
}

static void text_push_u64(Text *text, ULONGLONG value)
{
    WCHAR digits[20];
    SIZE_T count = 0;
    do {
        digits[19 - count] = (WCHAR)(L'0' + (value % 10));
        value /= 10;
        count++;
    } while (value != 0);
    text_push(text, digits + 20 - count, count);
}

static void text_push_hex32(Text *text, ULONG value)
{
    static const WCHAR hex[] = L"0123456789abcdef";
    WCHAR digits[8];
    int at;
    for (at = 7; at >= 0; at--) {
        digits[at] = hex[value & 0xF];
        value >>= 4;
    }
    text_push(text, digits, 8);
}

static void text_clear(Text *text)
{
    text->length = 0;
    if (text->chars != NULL) {
        text->chars[0] = 0;
    }
}

static void text_free(Text *text)
{
    release(text->chars);
    text->chars = NULL;
    text->length = 0;
    text->capacity = 0;
}

/* `<message> (os error <code>)`, the way the engine renders one. */
static void text_push_os_error(Text *text, DWORD code)
{
    WCHAR *message = NULL;
    DWORD length = FormatMessageW(
        FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
        NULL, code, 0, (LPWSTR)&message, 0, NULL);
    if (message != NULL) {
        while (length > 0 && (message[length - 1] == L'\r' || message[length - 1] == L'\n'
                              || message[length - 1] == L' ')) {
            length--;
        }
        text_push(text, message, length);
        LocalFree(message);
        text_push_ascii(text, " (os error ");
    } else {
        text_push_ascii(text, "(os error ");
    }
    text_push_u64(text, code);
    text_push_ascii(text, ")");
}

/* `cannot <verb> <path>: <last error>`. */
static void fail_with_os_error(Text *message, const char *verb, const WCHAR *path, DWORD code)
{
    text_clear(message);
    text_push_ascii(message, verb);
    text_push_ascii(message, " ");
    text_push_wide(message, path);
    text_push_ascii(message, ": ");
    text_push_os_error(message, code);
}

static void fail_with_text(Text *message, const char *text)
{
    text_clear(message);
    text_push_ascii(message, text);
}

/* --------------------------------------------------------------------- */
/* Console and reporting                                                  */
/* --------------------------------------------------------------------- */

static BOOL handle_missing(HANDLE handle)
{
    return handle == NULL || handle == INVALID_HANDLE_VALUE;
}

/* Opens the attached console's screen buffer. */
static HANDLE open_console(void)
{
    HANDLE handle = CreateFileW(L"CONOUT$", GENERIC_READ | GENERIC_WRITE,
                                FILE_SHARE_READ | FILE_SHARE_WRITE, NULL, OPEN_EXISTING,
                                FILE_ATTRIBUTE_NORMAL, NULL);
    return handle_missing(handle) ? NULL : handle;
}

/* Gives this process somewhere to print when it was started from a
 * terminal rather than with redirected output. Does nothing when output is
 * already redirected, and nothing when there is no parent console. A
 * process that already has a console cannot attach to another; the console
 * it has is the right one, and `CONOUT$` opens on it. */
static void attach_to_parent_console(void)
{
    BOOL out_missing = handle_missing(GetStdHandle(STD_OUTPUT_HANDLE));
    BOOL error_missing = handle_missing(GetStdHandle(STD_ERROR_HANDLE));
    HANDLE handle;
    if (!out_missing && !error_missing) {
        return;
    }
    AttachConsole(ATTACH_PARENT_PROCESS);
    if (out_missing && (handle = open_console()) != NULL) {
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
    }
    if (error_missing && (handle = open_console()) != NULL) {
        SetStdHandle(STD_ERROR_HANDLE, handle);
    }
}

/* Writes `text` to standard error: as UTF-16 when that is a console, as
 * UTF-8 when it is a pipe or a file, nowhere when there is none. */
static void write_standard_error(const Text *text)
{
    HANDLE handle = GetStdHandle(STD_ERROR_HANDLE);
    DWORD mode;
    DWORD written;
    int bytes;
    char *utf8;
    if (handle_missing(handle) || text->length == 0) {
        return;
    }
    if (GetConsoleMode(handle, &mode)) {
        WriteConsoleW(handle, text->chars, (DWORD)text->length, &written, NULL);
        return;
    }
    bytes = WideCharToMultiByte(CP_UTF8, 0, text->chars, (int)text->length, NULL, 0, NULL, NULL);
    if (bytes <= 0) {
        return;
    }
    utf8 = (char *)allocate((SIZE_T)bytes);
    if (utf8 == NULL) {
        return;
    }
    WideCharToMultiByte(CP_UTF8, 0, text->chars, (int)text->length, utf8, bytes, NULL, NULL);
    WriteFile(handle, utf8, (DWORD)bytes, &written, NULL);
    release(utf8);
}

/* --------------------------------------------------------------------- */
/* The command line                                                       */
/* --------------------------------------------------------------------- */

/* Whether `c` is white space as Unicode defines it, which is what the
 * engine's own trimming treats as blank. */
static BOOL is_whitespace(WCHAR c)
{
    return c == L' ' || (c >= 0x0009 && c <= 0x000D) || c == 0x0085 || c == 0x00A0
        || c == 0x1680 || (c >= 0x2000 && c <= 0x200A) || c == 0x2028 || c == 0x2029
        || c == 0x202F || c == 0x205F || c == 0x3000;
}

/* This process's command line after its own program name, verbatim, with
 * the white space that separated the two removed. The program name ends by
 * the rule the C runtime parses it with: a quoted token ends at the
 * closing quote, an unquoted one at the first space or tab. */
static const WCHAR *command_line_tail(void)
{
    const WCHAR *line = GetCommandLineW();
    if (line == NULL) {
        return L"";
    }
    if (*line == L'"') {
        line++;
        while (*line != 0 && *line != L'"') {
            line++;
        }
        if (*line == L'"') {
            line++;
        }
    } else {
        while (*line != 0 && *line != L' ' && *line != L'\t') {
            line++;
        }
    }
    while (*line != 0 && is_whitespace(*line)) {
        line++;
    }
    return line;
}

/* Whether the tail says anything at all: a double-click gives none. */
static BOOL tail_is_blank(const WCHAR *tail)
{
    while (*tail != 0) {
        if (!is_whitespace(*tail)) {
            return FALSE;
        }
        tail++;
    }
    return TRUE;
}

/* --------------------------------------------------------------------- */
/* The file and its footer                                                */
/* --------------------------------------------------------------------- */

typedef struct Footer {
    ULONGLONG engine_offset;
    ULONGLONG engine_length;
    ULONGLONG engine_uncompressed_length;
    ULONGLONG payload_offset;
    ULONGLONG payload_length;
    ULONGLONG payload_uncompressed_length;
    ULONGLONG metadata_offset;
    ULONGLONG metadata_length;
    ULONGLONG metadata_uncompressed_length;
    BYTE engine_sha256[32];
    BYTE engine_executable_sha256[32];
} Footer;

static ULONGLONG read_u64(const BYTE *at)
{
    ULONGLONG value = 0;
    int i;
    for (i = 7; i >= 0; i--) {
        value = (value << 8) | at[i];
    }
    return value;
}

static ULONG read_u32(const BYTE *at)
{
    return (ULONG)at[0] | ((ULONG)at[1] << 8) | ((ULONG)at[2] << 16) | ((ULONG)at[3] << 24);
}

static USHORT read_u16(const BYTE *at)
{
    return (USHORT)(at[0] | (at[1] << 8));
}

/* CRC-32 (IEEE 802.3, the polynomial zlib and crc32fast use), bit by bit:
 * the footer is 308 bytes, which needs no table. */
static ULONG crc32(const BYTE *bytes, SIZE_T count)
{
    ULONG crc = 0xFFFFFFFFu;
    SIZE_T i;
    int bit;
    for (i = 0; i < count; i++) {
        crc ^= bytes[i];
        for (bit = 0; bit < 8; bit++) {
            crc = (crc >> 1) ^ (0xEDB88320u & (0u - (crc & 1u)));
        }
    }
    return ~crc;
}

/* Reads exactly `count` bytes at `offset`; fewer is a failure. */
static BOOL read_exact(HANDLE file, ULONGLONG offset, BYTE *buffer, DWORD count, const WCHAR *path,
                       Text *message)
{
    LARGE_INTEGER position;
    DWORD filled = 0;
    position.QuadPart = (LONGLONG)offset;
    if (!SetFilePointerEx(file, position, NULL, FILE_BEGIN)) {
        fail_with_os_error(message, "cannot read", path, GetLastError());
        return FALSE;
    }
    while (filled < count) {
        DWORD got = 0;
        if (!ReadFile(file, buffer + filled, count - filled, &got, NULL)) {
            fail_with_os_error(message, "cannot read", path, GetLastError());
            return FALSE;
        }
        if (got == 0) {
            fail_with_text(message, "the file is shorter than its footer describes");
            return FALSE;
        }
        filled += got;
    }
    return TRUE;
}

/* Reads `IMAGE_DIRECTORY_ENTRY_SECURITY` (offset, size) from the PE headers
 * in `head`, if they are well formed. */
static BOOL security_directory(const BYTE *head, SIZE_T head_length, ULONG *offset, ULONG *size)
{
    SIZE_T pe;
    SIZE_T optional;
    SIZE_T directories;
    USHORT magic;
    if (head_length < 0x40 || read_u16(head) != 0x5A4D) {
        return FALSE;
    }
    pe = read_u32(head + 0x3C);
    if (pe + 4 > head_length || read_u32(head + pe) != 0x00004550) {
        return FALSE;
    }
    optional = pe + 24;
    if (optional + 2 > head_length) {
        return FALSE;
    }
    magic = read_u16(head + optional);
    if (magic == 0x20B) {
        directories = optional + 112;
    } else if (magic == 0x10B) {
        directories = optional + 96;
    } else {
        return FALSE;
    }
    if (directories + 4 * 8 + 8 > head_length) {
        return FALSE;
    }
    *offset = read_u32(head + directories + 4 * 8);
    *size = read_u32(head + directories + 4 * 8 + 4);
    return TRUE;
}

/* Where the footer starts: `file length - 320`, unless the executable
 * carries an Authenticode signature, whose certificate table follows the
 * footer, in which case the footer ends where that table begins. */
static BOOL locate_footer(HANDLE file, ULONGLONG file_length, const WCHAR *path,
                          ULONGLONG *footer_offset, Text *message)
{
    BYTE head[4096];
    DWORD head_length = 0;
    ULONGLONG end = file_length;
    ULONG security_offset;
    ULONG security_size;
    LARGE_INTEGER start;
    if (file_length < FOOTER_LEN) {
        fail_with_text(message, "the file is shorter than a footer");
        return FALSE;
    }
    start.QuadPart = 0;
    if (!SetFilePointerEx(file, start, NULL, FILE_BEGIN)) {
        fail_with_os_error(message, "cannot read", path, GetLastError());
        return FALSE;
    }
    while (head_length < sizeof head) {
        DWORD got = 0;
        if (!ReadFile(file, head + head_length, (DWORD)sizeof head - head_length, &got, NULL)) {
            fail_with_os_error(message, "cannot read", path, GetLastError());
            return FALSE;
        }
        if (got == 0) {
            break;
        }
        head_length += got;
    }
    if (security_directory(head, head_length, &security_offset, &security_size)
        && security_size > 0 && security_offset <= file_length) {
        end = security_offset;
    }
    if (end < FOOTER_LEN) {
        fail_with_text(message, "the file is shorter than a footer");
        return FALSE;
    }
    *footer_offset = end - FOOTER_LEN;
    return TRUE;
}

/* Decodes the footer bytes: magic, CRC-32, format, length, then the
 * fields; then checks that the blocks lie inside the file, in order —
 * loader, engine, payload, metadata — and end where the footer begins. */
static BOOL decode_footer(const BYTE *bytes, ULONGLONG footer_offset, Footer *footer, Text *message)
{
    ULONG expected_crc;
    ULONG actual_crc;
    USHORT format_major;
    USHORT format_minor;
    ULONG footer_length;
    ULONGLONG engine_end;
    ULONGLONG payload_end;
    ULONGLONG metadata_end;
    if (!bytes_equal(bytes, MAGIC_HEAD, 8) || !bytes_equal(bytes + MAGIC_TAIL_OFFSET, MAGIC_TAIL, 8)) {
        fail_with_text(message, "the file does not end with a TigerSetup footer");
        return FALSE;
    }
    expected_crc = read_u32(bytes + CRC_OFFSET);
    actual_crc = crc32(bytes, CRC_OFFSET);
    if (expected_crc != actual_crc) {
        fail_with_text(message, "footer CRC-32 is ");
        text_push_hex32(message, actual_crc);
        text_push_ascii(message, ", expected ");
        text_push_hex32(message, expected_crc);
        return FALSE;
    }
    format_major = read_u16(bytes + 8);
    format_minor = read_u16(bytes + 10);
    if (format_major != FORMAT_MAJOR) {
        fail_with_text(message, "installer format ");
        text_push_u64(message, format_major);
        text_push_ascii(message, ".");
        text_push_u64(message, format_minor);
        text_push_ascii(message, " is not supported by format ");
        text_push_u64(message, FORMAT_MAJOR);
        text_push_ascii(message, ".");
        text_push_u64(message, FORMAT_MINOR);
        return FALSE;
    }
    footer_length = read_u32(bytes + 12);
    if (footer_length != FOOTER_LEN) {
        fail_with_text(message, "footer length field is ");
        text_push_u64(message, footer_length);
        text_push_ascii(message, ", expected ");
        text_push_u64(message, FOOTER_LEN);
        return FALSE;
    }
    footer->engine_offset = read_u64(bytes + 16);
    footer->engine_length = read_u64(bytes + 24);
    footer->engine_uncompressed_length = read_u64(bytes + 32);
    footer->payload_offset = read_u64(bytes + 40);
    footer->payload_length = read_u64(bytes + 48);
    footer->payload_uncompressed_length = read_u64(bytes + 56);
    footer->metadata_offset = read_u64(bytes + 64);
    footer->metadata_length = read_u64(bytes + 72);
    footer->metadata_uncompressed_length = read_u64(bytes + 80);
    memcpy(footer->engine_sha256, bytes + 88, 32);
    memcpy(footer->engine_executable_sha256, bytes + 120, 32);

    engine_end = footer->engine_offset + footer->engine_length;
    payload_end = footer->payload_offset + footer->payload_length;
    metadata_end = footer->metadata_offset + footer->metadata_length;
    if (engine_end < footer->engine_offset || payload_end < footer->payload_offset
        || metadata_end < footer->metadata_offset || footer->engine_offset == 0
        || footer->engine_length == 0 || footer->engine_uncompressed_length == 0
        || engine_end != footer->payload_offset || payload_end != footer->metadata_offset
        || footer->metadata_length == 0 || footer->metadata_uncompressed_length == 0
        || metadata_end != footer_offset
        || (footer->payload_length == 0) != (footer->payload_uncompressed_length == 0)) {
        fail_with_text(message, "the footer's block map does not describe the file");
        return FALSE;
    }
    return TRUE;
}

static BOOL read_footer(HANDLE file, const WCHAR *path, Footer *footer, Text *message)
{
    LARGE_INTEGER size;
    ULONGLONG footer_offset;
    BYTE bytes[FOOTER_LEN];
    if (!GetFileSizeEx(file, &size)) {
        fail_with_os_error(message, "cannot read", path, GetLastError());
        return FALSE;
    }
    if (!locate_footer(file, (ULONGLONG)size.QuadPart, path, &footer_offset, message)) {
        return FALSE;
    }
    if (!read_exact(file, footer_offset, bytes, FOOTER_LEN, path, message)) {
        return FALSE;
    }
    return decode_footer(bytes, footer_offset, footer, message);
}

/* --------------------------------------------------------------------- */
/* SHA-256 through CNG                                                    */
/* --------------------------------------------------------------------- */

typedef struct Hash {
    BCRYPT_ALG_HANDLE algorithm;
    BCRYPT_HASH_HANDLE hash;
} Hash;

static BOOL hash_open(Hash *hash, Text *message)
{
    NTSTATUS status = BCryptOpenAlgorithmProvider(&hash->algorithm, BCRYPT_SHA256_ALGORITHM, NULL, 0);
    hash->hash = NULL;
    if (status < 0) {
        fail_with_text(message, "SHA-256 is unavailable (status 0x");
        text_push_hex32(message, (ULONG)status);
        text_push_ascii(message, ")");
        return FALSE;
    }
    return TRUE;
}

static BOOL hash_begin(Hash *hash, Text *message)
{
    NTSTATUS status;
    if (hash->hash != NULL) {
        BCryptDestroyHash(hash->hash);
        hash->hash = NULL;
    }
    status = BCryptCreateHash(hash->algorithm, &hash->hash, NULL, 0, NULL, 0, 0);
    if (status < 0) {
        hash->hash = NULL;
        fail_with_text(message, "SHA-256 is unavailable (status 0x");
        text_push_hex32(message, (ULONG)status);
        text_push_ascii(message, ")");
        return FALSE;
    }
    return TRUE;
}

static void hash_update(Hash *hash, const void *bytes, DWORD count)
{
    if (count > 0) {
        BCryptHashData(hash->hash, (PUCHAR)bytes, count, 0);
    }
}

static void hash_finish(Hash *hash, BYTE digest[32])
{
    BCryptFinishHash(hash->hash, digest, 32, 0);
    BCryptDestroyHash(hash->hash);
    hash->hash = NULL;
}

static void hash_close(Hash *hash)
{
    if (hash->hash != NULL) {
        BCryptDestroyHash(hash->hash);
        hash->hash = NULL;
    }
    if (hash->algorithm != NULL) {
        BCryptCloseAlgorithmProvider(hash->algorithm, 0);
        hash->algorithm = NULL;
    }
}

/* --------------------------------------------------------------------- */
/* Engine extraction                                                      */
/* --------------------------------------------------------------------- */

/* One decoded chunk on its way into the engine file: counted, hashed,
 * bounded by the footer, written. */
static BOOL emit(HANDLE out, const WCHAR *partial, const BYTE *bytes, DWORD count,
                 const Footer *footer, ULONGLONG *written, Hash *hash, Text *message)
{
    DWORD put = 0;
    if (count == 0) {
        return TRUE;
    }
    *written += count;
    if (*written > footer->engine_uncompressed_length) {
        fail_with_text(message, "the engine block decompresses to more than the footer declares");
        return FALSE;
    }
    hash_update(hash, bytes, count);
    if (!WriteFile(out, bytes, count, &put, NULL) || put != count) {
        fail_with_os_error(message, "cannot write", partial, GetLastError());
        return FALSE;
    }
    return TRUE;
}

/* libzstd's error strings are compiled out; the code is its error number. */
static void fail_decompression(Text *message, size_t code)
{
    fail_with_text(message, "the engine block cannot be decompressed (zstd error ");
    text_push_u64(message, (ULONGLONG)(0 - code));
    text_push_ascii(message, ")");
}

/* Decompresses the engine executable of `file`, mapped by `footer`, into
 * `out`, checking the compressed block's hash first and then the
 * decompressed length and hash against the footer. Nothing is trusted until
 * every check passed: the caller discards the file it is about to execute
 * on an error. */
static BOOL extract_engine(HANDLE file, const WCHAR *path, const Footer *footer, HANDLE out,
                           const WCHAR *partial, Text *message)
{
    BOOL ok = FALSE;
    Hash hash = {NULL, NULL};
    BYTE *in_buffer = NULL;
    BYTE *out_buffer = NULL;
    SIZE_T out_capacity;
    ZSTD_DStream *stream = NULL;
    BYTE digest[32];
    ULONGLONG remaining;
    ULONGLONG written = 0;
    size_t last = 1;

    if (!hash_open(&hash, message)) {
        goto done;
    }
    in_buffer = (BYTE *)allocate(READ_CHUNK);
    out_capacity = ZSTD_DStreamOutSize();
    out_buffer = (BYTE *)allocate(out_capacity);
    if (in_buffer == NULL || out_buffer == NULL) {
        fail_with_text(message, "out of memory");
        goto done;
    }

    /* The compressed block, against the footer. */
    if (!hash_begin(&hash, message)) {
        goto done;
    }
    remaining = footer->engine_length;
    {
        LARGE_INTEGER position;
        position.QuadPart = (LONGLONG)footer->engine_offset;
        if (!SetFilePointerEx(file, position, NULL, FILE_BEGIN)) {
            fail_with_os_error(message, "cannot read", path, GetLastError());
            goto done;
        }
    }
    while (remaining > 0) {
        DWORD want = remaining > READ_CHUNK ? READ_CHUNK : (DWORD)remaining;
        DWORD got = 0;
        if (!ReadFile(file, in_buffer, want, &got, NULL)) {
            fail_with_os_error(message, "cannot read", path, GetLastError());
            goto done;
        }
        if (got == 0) {
            fail_with_text(message, "the file is shorter than its footer describes");
            goto done;
        }
        hash_update(&hash, in_buffer, got);
        remaining -= got;
    }
    hash_finish(&hash, digest);
    if (!bytes_equal(digest, footer->engine_sha256, 32)) {
        fail_with_text(message, "the compressed engine block does not match the hash in the footer");
        goto done;
    }

    /* The executable it decompresses to, against the footer, as it is
     * written. */
    stream = ZSTD_createDStream();
    if (stream == NULL) {
        fail_with_text(message, "out of memory");
        goto done;
    }
    if (ZSTD_isError(ZSTD_DCtx_setParameter(stream, ZSTD_d_windowLogMax, MAX_WINDOW_LOG))) {
        fail_with_text(message, "the engine block cannot be decompressed (window limit refused)");
        goto done;
    }
    if (!hash_begin(&hash, message)) {
        goto done;
    }
    remaining = footer->engine_length;
    {
        LARGE_INTEGER position;
        position.QuadPart = (LONGLONG)footer->engine_offset;
        if (!SetFilePointerEx(file, position, NULL, FILE_BEGIN)) {
            fail_with_os_error(message, "cannot read", path, GetLastError());
            goto done;
        }
    }
    while (remaining > 0) {
        DWORD want = remaining > READ_CHUNK ? READ_CHUNK : (DWORD)remaining;
        DWORD got = 0;
        ZSTD_inBuffer in;
        if (!ReadFile(file, in_buffer, want, &got, NULL)) {
            fail_with_os_error(message, "cannot read", path, GetLastError());
            goto done;
        }
        if (got == 0) {
            fail_with_text(message, "the file is shorter than its footer describes");
            goto done;
        }
        remaining -= got;
        in.src = in_buffer;
        in.size = got;
        in.pos = 0;
        while (in.pos < in.size) {
            ZSTD_outBuffer o;
            o.dst = out_buffer;
            o.size = out_capacity;
            o.pos = 0;
            last = ZSTD_decompressStream(stream, &o, &in);
            if (ZSTD_isError(last)) {
                fail_decompression(message, last);
                goto done;
            }
            if (!emit(out, partial, out_buffer, (DWORD)o.pos, footer, &written, &hash, message)) {
                goto done;
            }
        }
    }
    /* The input is consumed; flush what the decoder still holds. A frame
     * that ends before its declared content is incomplete. */
    while (last != 0) {
        ZSTD_inBuffer in;
        ZSTD_outBuffer o;
        in.src = in_buffer;
        in.size = 0;
        in.pos = 0;
        o.dst = out_buffer;
        o.size = out_capacity;
        o.pos = 0;
        last = ZSTD_decompressStream(stream, &o, &in);
        if (ZSTD_isError(last)) {
            fail_decompression(message, last);
            goto done;
        }
        if (o.pos == 0) {
            break;
        }
        if (!emit(out, partial, out_buffer, (DWORD)o.pos, footer, &written, &hash, message)) {
            goto done;
        }
    }
    if (last != 0) {
        fail_with_text(message, "the engine block cannot be decompressed (the frame is incomplete)");
        goto done;
    }
    if (written != footer->engine_uncompressed_length) {
        fail_with_text(message, "the engine block decompresses to ");
        text_push_u64(message, written);
        text_push_ascii(message, " bytes, the footer declares ");
        text_push_u64(message, footer->engine_uncompressed_length);
        goto done;
    }
    hash_finish(&hash, digest);
    if (!bytes_equal(digest, footer->engine_executable_sha256, 32)) {
        fail_with_text(message, "the engine executable does not match the hash in the footer");
        goto done;
    }
    ok = TRUE;

done:
    if (stream != NULL) {
        ZSTD_freeDStream(stream);
    }
    release(out_buffer);
    release(in_buffer);
    hash_close(&hash);
    return ok;
}

/* --------------------------------------------------------------------- */
/* Directories                                                            */
/* --------------------------------------------------------------------- */

static BOOL path_exists(const WCHAR *path)
{
    return GetFileAttributesW(path) != INVALID_FILE_ATTRIBUTES;
}

/* Removes `path` and everything in it. A reparse point — a junction or a
 * symbolic link — is removed as an entry and never followed. Returns
 * whether the directory is gone. */
static BOOL remove_tree(Text *path)
{
    SIZE_T base = path->length;
    WIN32_FIND_DATAW entry;
    HANDLE find;
    text_push_ascii(path, "\\*");
    find = FindFirstFileExW(path->chars, FindExInfoBasic, &entry, FindExSearchNameMatch, NULL, 0);
    path->length = base;
    path->chars[base] = 0;
    if (find != INVALID_HANDLE_VALUE) {
        do {
            BOOL is_dot = entry.cFileName[0] == L'.'
                && (entry.cFileName[1] == 0 || (entry.cFileName[1] == L'.' && entry.cFileName[2] == 0));
            if (is_dot) {
                continue;
            }
            text_push_ascii(path, "\\");
            text_push_wide(path, entry.cFileName);
            if ((entry.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
                if ((entry.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) == 0) {
                    remove_tree(path);
                } else {
                    RemoveDirectoryW(path->chars);
                }
            } else {
                if (!DeleteFileW(path->chars)
                    && (entry.dwFileAttributes & FILE_ATTRIBUTE_READONLY) != 0) {
                    SetFileAttributesW(path->chars, FILE_ATTRIBUTE_NORMAL);
                    DeleteFileW(path->chars);
                }
            }
            path->length = base;
            path->chars[base] = 0;
        } while (FindNextFileW(find, &entry));
        FindClose(find);
    }
    if (RemoveDirectoryW(path->chars)) {
        return TRUE;
    }
    return !path_exists(path->chars);
}

/* Removes the extraction directory and everything in it. An antivirus may
 * still hold the engine open for a moment after it exited, so the removal
 * is retried briefly; what cannot be removed is left, harmless, in a
 * folder of this launch's own. */
static void remove_directory(Text *directory)
{
    DWORD attempt;
    for (attempt = 0; attempt < 10; attempt++) {
        if (remove_tree(directory)) {
            return;
        }
        Sleep(100 * (attempt + 1));
    }
}

/* Removes extraction directories a killed launch left under the user's
 * root, once they are a day old: young ones may belong to a launch still
 * running. */
static void sweep_stale(const Text *root)
{
    Text pattern = {NULL, 0, 0};
    WIN32_FIND_DATAW entry;
    HANDLE find;
    FILETIME now_time;
    ULONGLONG now;
    GetSystemTimeAsFileTime(&now_time);
    now = ((ULONGLONG)now_time.dwHighDateTime << 32) | now_time.dwLowDateTime;
    text_push(&pattern, root->chars, root->length);
    text_push_ascii(&pattern, "\\*");
    find = FindFirstFileExW(pattern.chars, FindExInfoBasic, &entry, FindExSearchNameMatch, NULL, 0);
    if (find != INVALID_HANDLE_VALUE) {
        do {
            ULONGLONG modified = ((ULONGLONG)entry.ftLastWriteTime.dwHighDateTime << 32)
                | entry.ftLastWriteTime.dwLowDateTime;
            BOOL is_dot = entry.cFileName[0] == L'.'
                && (entry.cFileName[1] == 0 || (entry.cFileName[1] == L'.' && entry.cFileName[2] == 0));
            if (is_dot || (entry.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) == 0) {
                continue;
            }
            if (now > modified && now - modified > STALE_AGE) {
                text_clear(&pattern);
                text_push(&pattern, root->chars, root->length);
                text_push_ascii(&pattern, "\\");
                text_push_wide(&pattern, entry.cFileName);
                remove_tree(&pattern);
            }
        } while (FindNextFileW(find, &entry));
        FindClose(find);
    }
    text_free(&pattern);
}

/* Creates `path` and every missing directory above it. */
static BOOL create_directory_all(const Text *path, Text *message)
{
    Text prefix = {NULL, 0, 0};
    SIZE_T at;
    BOOL ok = TRUE;
    if (CreateDirectoryW(path->chars, NULL) || GetLastError() == ERROR_ALREADY_EXISTS) {
        return TRUE;
    }
    /* Each component after the root, in order; one that exists already or
     * cannot be created is passed over, and the final call reports the
     * failure that matters. */
    for (at = 0; at < path->length; at++) {
        if ((path->chars[at] == L'\\' || path->chars[at] == L'/') && at > 2) {
            text_clear(&prefix);
            text_push(&prefix, path->chars, at);
            CreateDirectoryW(prefix.chars, NULL);
        }
    }
    text_free(&prefix);
    if (!CreateDirectoryW(path->chars, NULL) && GetLastError() != ERROR_ALREADY_EXISTS) {
        fail_with_os_error(message, "cannot create", path->chars, GetLastError());
        ok = FALSE;
    }
    return ok;
}

/* Whether this process runs with an elevated token. */
static BOOL is_elevated(void)
{
    HANDLE token = NULL;
    TOKEN_ELEVATION elevation;
    DWORD returned = 0;
    BOOL ok;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) {
        return FALSE;
    }
    elevation.TokenIsElevated = 0;
    ok = GetTokenInformation(token, TokenElevation, &elevation, sizeof elevation, &returned);
    CloseHandle(token);
    return ok && elevation.TokenIsElevated != 0;
}

/* Creates `directory`, which must not exist. An elevated run creates it
 * with ELEVATED_DACL in the same call, so there is no moment at which it
 * exists with a weaker list. Returns the Win32 error on failure, 0 on
 * success. */
static DWORD create_directory(const WCHAR *directory, BOOL elevated)
{
    SECURITY_ATTRIBUTES attributes;
    PSECURITY_DESCRIPTOR descriptor = NULL;
    DWORD error = 0;
    if (!elevated) {
        return CreateDirectoryW(directory, NULL) ? 0 : GetLastError();
    }
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(ELEVATED_DACL, SDDL_REVISION_1,
                                                              &descriptor, NULL)) {
        return GetLastError();
    }
    attributes.nLength = sizeof attributes;
    attributes.lpSecurityDescriptor = descriptor;
    attributes.bInheritHandle = FALSE;
    if (!CreateDirectoryW(directory, &attributes)) {
        error = GetLastError();
    }
    LocalFree(descriptor);
    return error;
}

/* The directory a system call fills: `%TEMP%\` or `%SystemRoot%`. */
static BOOL push_system_path(Text *text, DWORD (WINAPI *query)(DWORD, LPWSTR))
{
    DWORD capacity = MAX_PATH;
    for (;;) {
        DWORD length;
        text_reserve(text, capacity);
        length = query(capacity, text->chars + text->length);
        if (length == 0) {
            return FALSE;
        }
        if (length < capacity) {
            text->length += length;
            text->chars[text->length] = 0;
            return TRUE;
        }
        capacity = length + 1;
    }
}

static DWORD WINAPI query_temp_path(DWORD capacity, LPWSTR buffer)
{
    return GetTempPathW(capacity, buffer);
}

static DWORD WINAPI query_windows_directory(DWORD capacity, LPWSTR buffer)
{
    return GetWindowsDirectoryW(buffer, capacity);
}

/* A fresh directory of this launch's own, for the engine to run from. */
static BOOL extraction_directory(Text *directory, Text *message)
{
    BOOL elevated = is_elevated();
    Text root = {NULL, 0, 0};
    DWORD pid = GetCurrentProcessId();
    DWORD attempt = 0;
    BOOL ok = FALSE;

    if (elevated) {
        if (!push_system_path(&root, query_windows_directory)) {
            fail_with_os_error(message, "cannot locate", L"the Windows directory", GetLastError());
            goto done;
        }
        if (root.length > 0 && root.chars[root.length - 1] != L'\\') {
            text_push_ascii(&root, "\\");
        }
        text_push_ascii(&root, "Temp");
    } else {
        if (!push_system_path(&root, query_temp_path)) {
            fail_with_os_error(message, "cannot locate", L"the temporary directory", GetLastError());
            goto done;
        }
        if (root.length > 0 && root.chars[root.length - 1] != L'\\') {
            text_push_ascii(&root, "\\");
        }
        text_push_ascii(&root, "TigerSetup");
        if (!create_directory_all(&root, message)) {
            goto done;
        }
        sweep_stale(&root);
    }

    for (;;) {
        ULONGLONG tick = GetTickCount64();
        DWORD error;
        text_clear(directory);
        text_push(directory, root.chars, root.length);
        text_push_ascii(directory, elevated ? "\\TigerSetup-" : "\\");
        text_push_u64(directory, pid);
        text_push_ascii(directory, "-");
        text_push_u64(directory, tick);
        text_push_ascii(directory, "-");
        text_push_u64(directory, attempt);
        error = create_directory(directory->chars, elevated);
        if (error == 0) {
            ok = TRUE;
            break;
        }
        if (error == ERROR_ALREADY_EXISTS && attempt < 16) {
            attempt++;
            continue;
        }
        fail_with_os_error(message, "cannot create", directory->chars, error);
        break;
    }

done:
    text_free(&root);
    return ok;
}

/* --------------------------------------------------------------------- */
/* The engine                                                             */
/* --------------------------------------------------------------------- */

/* The path of this executable. */
static BOOL module_path(Text *path, Text *message)
{
    DWORD capacity = MAX_PATH;
    for (;;) {
        DWORD length;
        text_reserve(path, capacity);
        length = GetModuleFileNameW(NULL, path->chars, capacity);
        if (length == 0) {
            fail_with_os_error(message, "cannot locate", L"this executable", GetLastError());
            return FALSE;
        }
        if (length < capacity) {
            path->length = length;
            return TRUE;
        }
        if (capacity >= 32768) {
            fail_with_text(message, "cannot locate this executable: the path is too long");
            return FALSE;
        }
        capacity *= 2;
    }
}

/* The last component of `path`. */
static const WCHAR *file_name_of(const Text *path)
{
    SIZE_T at = path->length;
    while (at > 0 && path->chars[at - 1] != L'\\' && path->chars[at - 1] != L'/') {
        at--;
    }
    return path->chars + at;
}

/* Duplicates a standard handle so the child inherits it, the way a
 * process started with inherited standard streams gets them. A handle
 * that is already inheritable is passed as it is. */
static HANDLE inheritable(HANDLE handle, BOOL *duplicated)
{
    DWORD flags = 0;
    HANDLE copy = NULL;
    *duplicated = FALSE;
    if (handle_missing(handle)) {
        return NULL;
    }
    if (GetHandleInformation(handle, &flags) && (flags & HANDLE_FLAG_INHERIT) != 0) {
        return handle;
    }
    if (!DuplicateHandle(GetCurrentProcess(), handle, GetCurrentProcess(), &copy, 0, TRUE,
                         DUPLICATE_SAME_ACCESS)) {
        return NULL;
    }
    *duplicated = TRUE;
    return copy;
}

/* Starts the engine with this process's command line, verbatim, plus the
 * package, waits and returns its exit code. */
static BOOL start_engine(const WCHAR *engine, const WCHAR *tail, const Text *package, DWORD *code,
                         Text *message)
{
    Text line = {NULL, 0, 0};
    STARTUPINFOW startup;
    PROCESS_INFORMATION process;
    HANDLE handles[3];
    BOOL duplicated[3];
    DWORD ids[3] = {STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE};
    BOOL any = FALSE;
    BOOL ok = FALSE;
    int i;

    text_push_ascii(&line, "\"");
    text_push_wide(&line, engine);
    text_push_ascii(&line, "\"");
    if (!tail_is_blank(tail)) {
        text_push_ascii(&line, " ");
        text_push_wide(&line, tail);
    }
    text_push_ascii(&line, " ");
    text_push_wide(&line, PACKAGE_ARGUMENT);
    text_push_ascii(&line, " \"");
    text_push(&line, package->chars, package->length);
    text_push_ascii(&line, "\"");

    memset(&startup, 0, sizeof startup);
    startup.cb = sizeof startup;
    for (i = 0; i < 3; i++) {
        handles[i] = inheritable(GetStdHandle(ids[i]), &duplicated[i]);
        any = any || handles[i] != NULL;
    }
    if (any) {
        startup.dwFlags = STARTF_USESTDHANDLES;
        startup.hStdInput = handles[0];
        startup.hStdOutput = handles[1];
        startup.hStdError = handles[2];
    }
    memset(&process, 0, sizeof process);
    if (CreateProcessW(engine, line.chars, NULL, NULL, TRUE, 0, NULL, NULL, &startup, &process)) {
        CloseHandle(process.hThread);
        WaitForSingleObject(process.hProcess, INFINITE);
        if (!GetExitCodeProcess(process.hProcess, code)) {
            *code = INVALID;
        }
        CloseHandle(process.hProcess);
        ok = TRUE;
    } else {
        fail_with_os_error(message, "cannot start", engine, GetLastError());
    }
    for (i = 0; i < 3; i++) {
        if (duplicated[i]) {
            CloseHandle(handles[i]);
        }
    }
    text_free(&line);
    return ok;
}

/* Extracts the engine into `directory` and runs it. */
static BOOL run_engine(HANDLE file, const Text *package, const Footer *footer, const Text *directory,
                       const WCHAR *tail, DWORD *code, Text *message)
{
    Text engine = {NULL, 0, 0};
    Text partial = {NULL, 0, 0};
    HANDLE out;
    BOOL ok = FALSE;

    text_push(&engine, directory->chars, directory->length);
    text_push_ascii(&engine, "\\");
    text_push_wide(&engine, file_name_of(package));
    text_push(&partial, engine.chars, engine.length);
    text_push_ascii(&partial, ".partial");

    out = CreateFileW(partial.chars, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL,
                      NULL);
    if (out == INVALID_HANDLE_VALUE) {
        fail_with_os_error(message, "cannot create", partial.chars, GetLastError());
        goto done;
    }
    ok = extract_engine(file, package->chars, footer, out, partial.chars, message);
    CloseHandle(out);
    if (!ok) {
        goto done;
    }
    /* The file is the engine only once it is complete and verified; a
     * crash before this rename leaves a `.partial` that nothing executes. */
    if (!MoveFileExW(partial.chars, engine.chars, MOVEFILE_REPLACE_EXISTING)) {
        fail_with_os_error(message, "cannot place", engine.chars, GetLastError());
        ok = FALSE;
        goto done;
    }
    ok = start_engine(engine.chars, tail, package, code, message);

done:
    text_free(&partial);
    text_free(&engine);
    return ok;
}

/* Extracts the engine, runs it and cleans up. The exit code is the
 * engine's. */
static BOOL bootstrap(const WCHAR *tail, Text *package, DWORD *code, Text *message)
{
    HANDLE file;
    Footer footer;
    Text directory = {NULL, 0, 0};
    BOOL ok;

    if (!module_path(package, message)) {
        return FALSE;
    }
    file = CreateFileW(package->chars, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                       NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (file == INVALID_HANDLE_VALUE) {
        fail_with_os_error(message, "cannot open", package->chars, GetLastError());
        return FALSE;
    }
    ok = read_footer(file, package->chars, &footer, message);
    if (ok) {
        ok = extraction_directory(&directory, message);
        if (ok) {
            ok = run_engine(file, package, &footer, &directory, tail, code, message);
            remove_directory(&directory);
        }
    }
    CloseHandle(file);
    text_free(&directory);
    return ok;
}

static DWORD run(void)
{
    const WCHAR *tail;
    Text package = {NULL, 0, 0};
    Text message = {NULL, 0, 0};
    Text report = {NULL, 0, 0};
    DWORD code = INVALID;

    /* Load system DLLs from System32 only, before anything is loaded on
     * demand: an installer runs from a download folder, which is exactly
     * the application directory a planted DLL would be searched in first. */
    SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
    attach_to_parent_console();

    tail = command_line_tail();
    if (bootstrap(tail, &package, &code, &message)) {
        text_free(&message);
        text_free(&package);
        return code;
    }

    if (package.length > 0) {
        text_push(&report, package.chars, package.length);
    } else {
        text_push_ascii(&report, "this installer");
    }
    text_push_ascii(&report, " cannot start: ");
    text_push(&report, message.chars, message.length);
    {
        Text line = {NULL, 0, 0};
        text_push_ascii(&line, "error: ");
        text_push(&line, report.chars, report.length);
        text_push_ascii(&line, "\n");
        write_standard_error(&line);
        text_free(&line);
    }
    if (tail_is_blank(tail)) {
        MessageBoxW(NULL, report.chars, L"Setup", MB_OK | MB_ICONERROR);
    }
    text_free(&report);
    text_free(&message);
    text_free(&package);
    return INVALID;
}

/* --------------------------------------------------------------------- */
/* Entry                                                                  */
/* --------------------------------------------------------------------- */

#if defined(LOADER_CRT_STARTUP)

/* The C runtime's own start-up, then the loader. */
int WINAPI wWinMain(HINSTANCE instance, HINSTANCE previous, PWSTR arguments, int show)
{
    (void)instance;
    (void)previous;
    (void)arguments;
    (void)show;
    return (int)run();
}

#else

/* The loader as the process entry point: no C runtime start-up runs, so
 * the `/GS` cookie is seeded here, first, and the process ends explicitly.
 * Nothing else the C runtime initializes is used. */
void __cdecl __security_init_cookie(void);

void __cdecl LoaderEntry(void)
{
    __security_init_cookie();
    ExitProcess(run());
}

#endif
