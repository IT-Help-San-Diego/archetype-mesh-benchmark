#include <sys/stat.h>
#include <unistd.h>
#include <fcntl.h>

int read_file(const char *path, char *buf, int cap) {
    int fd = open(path, O_RDONLY);
    if (fd < 0) return 0;
    ssize_t n = read(fd, buf, cap);
    close(fd);
    if (n < 0) return 0;
    return (int)n;
}
