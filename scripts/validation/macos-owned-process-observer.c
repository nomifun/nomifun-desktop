// Native evidence helper for the isolated Desktop lifecycle driver. Each
// original process is bound to a Darwin generation and an EVFILT_PROC knote.
// Only argv[1]'s already captured generation can receive INT/TERM/KILL on stdin;
// this helper never enumerates or signals a process group or process name.
#include <sys/event.h>
#include <sys/proc_info.h>
#include <sys/types.h>
#include <libproc.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

struct unique_info {
  uint8_t executable_uuid[16];
  uint64_t unique_id, parent_unique_id;
  int32_t pid_version, original_parent_pid_version;
  uint64_t reserved[2];
};
struct observed { pid_t pid; pid_t parent_pid; struct unique_info identity; int exited; };
struct audit_token { uint32_t values[8]; };
typedef int (*signal_generation_fn)(struct audit_token *, int);

static void fail(const char *reason) {
  fprintf(stderr, "MACOS_OWNED_PROCESS_OBSERVER %s errno=%d\n", reason, errno);
  exit(1);
}
static struct unique_info identity(pid_t pid) {
  struct unique_info value;
  int size = (int)sizeof(value);
  if (proc_pidinfo(pid, 17 /* PROC_PIDUNIQIDENTIFIERINFO */, 1, &value, size) != size || !value.unique_id)
    fail("generation_read_failed");
  return value;
}
static int same(struct unique_info a, struct unique_info b) {
  return a.unique_id == b.unique_id && a.pid_version == b.pid_version;
}
static pid_t verify_program(pid_t pid, const char *expected) {
  char actual[PROC_PIDPATHINFO_MAXSIZE], physical[PATH_MAX];
  struct proc_bsdinfo bsd;
  if (proc_pidpath(pid, actual, sizeof(actual)) <= 0 || !realpath(actual, physical) || strcmp(physical, expected))
    fail("exact_program_mismatch");
  if (proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &bsd, sizeof(bsd)) != sizeof(bsd)
      || bsd.pbi_pid != (uint32_t)pid || bsd.pbi_uid != geteuid() || bsd.pbi_ruid != getuid())
    fail("owned_user_mismatch");
  return (pid_t)bsd.pbi_ppid;
}
int main(int argc, char **argv) {
  if (argc < 3 || argc > 33 || argc % 2 != 1) fail("usage_pid_program_pairs");
  setvbuf(stdout, NULL, _IOLBF, 0);
  signal_generation_fn signal_exact = (signal_generation_fn)dlsym(RTLD_DEFAULT, "proc_signal_with_audittoken");
  if (!signal_exact) fail("generation_signal_unavailable");
  int queue = kqueue();
  if (queue < 0 || fcntl(queue, F_SETFD, FD_CLOEXEC) < 0) fail("kqueue_open_failed");
  int count = (argc - 1) / 2;
  struct observed processes[16] = {0};
  for (int index = 0; index < count; index++) {
    char *end = NULL;
    long value = strtol(argv[index * 2 + 1], &end, 10);
    if (!end || *end || value <= 1 || value > INT32_MAX) fail("invalid_owned_pid");
    struct observed *process = &processes[index];
    process->pid = (pid_t)value;
    process->identity = identity(process->pid);
    process->parent_pid = verify_program(process->pid, argv[index * 2 + 2]);
    struct kevent request, receipt;
    EV_SET(&request, process->pid, EVFILT_PROC, EV_ADD | EV_ENABLE | EV_ONESHOT | EV_RECEIPT,
      NOTE_EXIT | NOTE_EXITSTATUS, 0, (void *)(intptr_t)index);
    struct timespec immediate = {0, 0};
    if (kevent(queue, &request, 1, &receipt, 1, &immediate) != 1 || receipt.ident != (uintptr_t)process->pid
        || receipt.filter != EVFILT_PROC || !(receipt.flags & EV_ERROR) || receipt.data != 0)
      fail("exact_exit_registration_failed");
    if (!same(process->identity, identity(process->pid))) fail("generation_changed_during_registration");
    printf("{\"event\":\"registered\",\"index\":%d,\"pid\":%d,\"unique_id\":\"%llu\",\"pid_version\":%d,\"parent_pid\":%d,\"parent_unique_id\":\"%llu\",\"original_parent_pid_version\":%d}\n",
      index, process->pid, (unsigned long long)process->identity.unique_id, process->identity.pid_version,
      process->parent_pid, (unsigned long long)process->identity.parent_unique_id, process->identity.original_parent_pid_version);
  }
  // Nonblocking reads allow the original NOTE_EXIT events to remain serviced
  // while the JS owner waits for GUI actions or the authenticated controller.
  if (fcntl(STDIN_FILENO, F_SETFL, O_NONBLOCK) < 0) fail("stdin_nonblocking_failed");
  printf("{\"event\":\"ready\",\"processes\":%d}\n", count);
  char input[128] = {0}; size_t used = 0;
  int exited = 0, requested = 0;
  while (exited < count) {
    struct kevent events[16]; struct timespec interval = {0, 50000000};
    int received = kevent(queue, NULL, 0, events, 16, &interval);
    if (received < 0 && errno == EINTR) continue;
    if (received < 0) fail("native_exit_wait_failed");
    for (int event = 0; event < received; event++) {
      int index = (int)(intptr_t)events[event].udata;
      if (index < 0 || index >= count || events[event].filter != EVFILT_PROC
          || !(events[event].fflags & NOTE_EXIT) || events[event].ident != (uintptr_t)processes[index].pid
          || processes[index].exited) fail("unexpected_native_exit_event");
      processes[index].exited = 1; exited++;
      printf("{\"event\":\"exit\",\"index\":%d,\"pid\":%d,\"note_exit\":true,\"note_exit_status\":%s,\"native_status\":%ld}\n",
        index, processes[index].pid, events[event].fflags & NOTE_EXITSTATUS ? "true" : "false", (long)events[event].data);
    }
    if (exited == count) break;
    ssize_t length = read(STDIN_FILENO, input + used, sizeof(input) - used - 1);
    if (length > 0) {
      used += (size_t)length; input[used] = 0;
      char *newline = strchr(input, '\n');
      if (newline) {
        *newline = 0;
        int signal = !strcmp(input, "SIGINT") ? SIGINT : !strcmp(input, "SIGTERM") ? SIGTERM : !strcmp(input, "SIGKILL") ? SIGKILL : 0;
        if (!signal || requested || processes[0].exited) fail("invalid_or_repeated_signal_request");
        struct audit_token token = {{0}};
        token.values[5] = (uint32_t)processes[0].pid;
        token.values[7] = (uint32_t)processes[0].identity.pid_version;
        int result = signal_exact(&token, signal);
        if (result) { errno = result; fail("exact_generation_signal_failed"); }
        requested = signal;
        printf("{\"event\":\"signal\",\"pid\":%d,\"signal\":%d,\"kernel_generation_checked\":true}\n", processes[0].pid, signal);
        used = 0; input[0] = 0;
      } else if (used == sizeof(input) - 1) fail("signal_request_too_large");
    } else if (length == 0 && !processes[0].exited) fail("owner_stdin_closed_before_host_exit");
    else if (length < 0 && errno != EAGAIN && errno != EINTR) fail("owner_stdin_read_failed");
  }
  printf("{\"event\":\"complete\",\"processes\":%d,\"all_original_note_exit\":true}\n", count);
  close(queue);
  return 0;
}
