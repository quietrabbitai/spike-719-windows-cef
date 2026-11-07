// items.id=720 (THROWAWAY): is each process of the spike's tree sandboxed? Uses sandbox_check() from libSystem
// (operation NULL, filter NONE: non-zero when the process is under a sandbox profile). NOTE: it is NOT declared in
// the public SDK headers (it lives in sandbox/private.h), so it is declared by hand below; hence the controls.
// usage: sandbox_check <pid> [<pid> ...]   prints "pid sandboxed=<0|1>"
#include <sys/types.h>
#include <stdio.h>
#include <stdlib.h>
#define SANDBOX_FILTER_NONE 0
extern int sandbox_check(pid_t pid, const char *operation, int type, ...);
int main(int argc, char **argv) {
  for (int i = 1; i < argc; i++) {
    pid_t pid = (pid_t)atoi(argv[i]);
    int r = sandbox_check(pid, NULL, SANDBOX_FILTER_NONE);
    printf("%d sandboxed=%d\n", pid, r != 0);
  }
  return 0;
}
