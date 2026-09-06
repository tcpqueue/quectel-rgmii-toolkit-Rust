#include <netdb.h>
#include <stdlib.h>
#include <unistd.h>
#include <stdio.h>
int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **res) {
    (void)service; (void)hints; (void)res;
    fprintf(stderr, "DNS start: %s\n", node);
    const char *delay = getenv("TEST_DNS_DELAY");
    sleep(delay ? atoi(delay) : 0);
    return EAI_AGAIN;
}
