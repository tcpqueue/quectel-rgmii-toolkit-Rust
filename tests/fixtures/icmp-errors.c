#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <unistd.h>

/* Local ICMP only. Leave HTTP, DNS and the AT worker untouched. */
int socket(int domain, int type, int protocol) {
    static int (*real_socket)(int, int, int);
    if (!real_socket) real_socket = dlsym(RTLD_NEXT, "socket");
    if (getenv("TEST_ICMP_RAW") && (protocol == 1 || protocol == 58)
        && (type & 15) == SOCK_DGRAM) {
        errno = EACCES;
        return -1;
    }
    return real_socket(domain, type, protocol);
}

ssize_t recvfrom(int fd, void *buf, size_t len, int flags,
                 struct sockaddr *addr, socklen_t *addrlen) {
    static ssize_t (*real_recvfrom)(int, void *, size_t, int,
                                   struct sockaddr *, socklen_t *);
    static unsigned long failures;
    if (!real_recvfrom) real_recvfrom = dlsym(RTLD_NEXT, "recvfrom");
    const char *fault = getenv("TEST_ICMP_FAULT_FILE");
    const char *drop = getenv("TEST_ICMP_DROP_FILE");
    int protocol = 0;
    socklen_t size = sizeof(protocol);
    if (getsockopt(fd, SOL_SOCKET, SO_PROTOCOL, &protocol, &size) == 0
        && (protocol == 1 || protocol == 58)) {
        if (fault && access(fault, F_OK) == 0) {
            failures++;
            if (failures <= 100) fprintf(stderr, "ICMP_RECV_ERROR %lu\n", failures);
            errno = ENETDOWN;
            return -1;
        }
        if (drop && access(drop, F_OK) == 0) {
            ssize_t n = real_recvfrom(fd, buf, len, flags, addr, addrlen);
            if (n >= 0) {
                fprintf(stderr, "ICMP_DROP\n");
                errno = EAGAIN;
                return -1;
            }
            return n;
        }
    }
    return real_recvfrom(fd, buf, len, flags, addr, addrlen);
}
