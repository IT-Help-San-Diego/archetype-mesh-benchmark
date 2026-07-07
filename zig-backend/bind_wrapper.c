#include <sys/socket.h>
#include <netinet/in.h>

int bind_in(int fd, const struct sockaddr_in *addr, socklen_t len) {
    return bind(fd, (const struct sockaddr *)addr, len);
}
