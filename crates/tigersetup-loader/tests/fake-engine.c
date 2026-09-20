/*
 * The engine the loader tests extract and run: a console program that
 * reports what the loader gave it and exits how it is told.
 *
 * Standard output, UTF-8, one line each:
 *
 *   command line: <GetCommandLineW, verbatim>
 *   executable: <GetModuleFileNameW>
 *   elevated: 0|1
 *   security: owner=<sid> protected=0|1 ace=<type>:<flags>:<mask>:<sid> ...
 *              (the directory the executable lies in, rights as numbers)
 *
 * `--exit <n>` sets the exit code (0 by default). `--hold <ms>` sleeps
 * after the lines are written and before exiting, so a test can observe
 * the extraction directory while the engine runs.
 */

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <aclapi.h>
#include <sddl.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

static void write_wide(const wchar_t *text)
{
    HANDLE out = GetStdHandle(STD_OUTPUT_HANDLE);
    int length = (int)wcslen(text);
    int bytes = WideCharToMultiByte(CP_UTF8, 0, text, length, NULL, 0, NULL, NULL);
    char *utf8 = (char *)malloc((size_t)bytes + 1);
    DWORD written = 0;
    if (utf8 == NULL || bytes <= 0) {
        return;
    }
    WideCharToMultiByte(CP_UTF8, 0, text, length, utf8, bytes, NULL, NULL);
    WriteFile(out, utf8, (DWORD)bytes, &written, NULL);
    free(utf8);
}

static void write_ascii(const char *text)
{
    HANDLE out = GetStdHandle(STD_OUTPUT_HANDLE);
    DWORD written = 0;
    WriteFile(out, text, (DWORD)strlen(text), &written, NULL);
}

static void write_hex(DWORD value)
{
    char text[16];
    int i;
    for (i = 7; i >= 0; i--) {
        text[i] = "0123456789abcdef"[value & 0xF];
        value >>= 4;
    }
    text[8] = 0;
    write_ascii(text);
}

static void write_sid(PSID sid)
{
    wchar_t *text = NULL;
    if (sid != NULL && ConvertSidToStringSidW(sid, &text)) {
        write_wide(text);
        LocalFree(text);
    } else {
        write_ascii("?");
    }
}

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

/* The owner and the discretionary list of `directory`, as numbers. */
static void write_security(wchar_t *directory)
{
    PSECURITY_DESCRIPTOR descriptor = NULL;
    PSID owner = NULL;
    PACL dacl = NULL;
    SECURITY_DESCRIPTOR_CONTROL control = 0;
    DWORD revision = 0;
    DWORD result = GetNamedSecurityInfoW(directory, SE_FILE_OBJECT,
                                         OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                                         &owner, NULL, &dacl, NULL, &descriptor);
    if (result != ERROR_SUCCESS) {
        write_ascii("security: unavailable\n");
        return;
    }
    GetSecurityDescriptorControl(descriptor, &control, &revision);
    write_ascii("security: owner=");
    write_sid(owner);
    write_ascii((control & SE_DACL_PROTECTED) != 0 ? " protected=1" : " protected=0");
    if (dacl != NULL) {
        DWORD index;
        for (index = 0; index < dacl->AceCount; index++) {
            ACE_HEADER *header = NULL;
            if (!GetAce(dacl, index, (LPVOID *)&header)) {
                continue;
            }
            write_ascii(" ace=");
            write_hex(header->AceType);
            write_ascii(":");
            write_hex(header->AceFlags);
            write_ascii(":");
            if (header->AceType == ACCESS_ALLOWED_ACE_TYPE) {
                ACCESS_ALLOWED_ACE *ace = (ACCESS_ALLOWED_ACE *)header;
                write_hex(ace->Mask);
                write_ascii(":");
                write_sid((PSID)&ace->SidStart);
            } else if (header->AceType == ACCESS_DENIED_ACE_TYPE) {
                ACCESS_DENIED_ACE *ace = (ACCESS_DENIED_ACE *)header;
                write_hex(ace->Mask);
                write_ascii(":");
                write_sid((PSID)&ace->SidStart);
            } else {
                write_ascii("?:?");
            }
        }
    }
    write_ascii("\n");
    LocalFree(descriptor);
}

int wmain(int argc, wchar_t **argv)
{
    wchar_t path[32768];
    DWORD length;
    int exit_code = 0;
    DWORD hold = 0;
    int i;

    write_ascii("command line: ");
    write_wide(GetCommandLineW());
    write_ascii("\n");

    length = GetModuleFileNameW(NULL, path, 32768);
    path[length] = 0;
    write_ascii("executable: ");
    write_wide(path);
    write_ascii("\n");

    write_ascii(is_elevated() ? "elevated: 1\n" : "elevated: 0\n");

    /* The directory the executable lies in. */
    while (length > 0 && path[length - 1] != L'\\') {
        length--;
    }
    if (length > 1) {
        path[length - 1] = 0;
    }
    write_security(path);

    for (i = 1; i < argc; i++) {
        if (wcscmp(argv[i], L"--exit") == 0 && i + 1 < argc) {
            exit_code = _wtoi(argv[++i]);
        } else if (wcscmp(argv[i], L"--hold") == 0 && i + 1 < argc) {
            hold = (DWORD)_wtoi(argv[++i]);
        }
    }
    if (hold > 0) {
        Sleep(hold);
    }
    return exit_code;
}
