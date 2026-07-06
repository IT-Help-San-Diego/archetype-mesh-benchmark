const std = @import("std");
const c = @cImport({
    @cInclude("unistd.h");
    @cInclude("sys/socket.h");
    @cInclude("netinet/in.h");
    @cInclude("arpa/inet.h");
    @cInclude("string.h");
});

fn handle_client(client_fd: c_int) void {
    const response =
        \\HTTP/1.1 200 OK\r
        \\Content-Type: text/plain\r
        \\Connection: close\r
        \\\r
        \\Zig foundation: alive\r
    ;
    _ = c.send(client_fd, response, c.strlen(response), 0);
    _ = c.close(client_fd);
}

pub fn main() void {
    const listen_fd = c.socket(c.AF_INET, c.SOCK_STREAM, 0);
    if (listen_fd < 0) {
        std.debug.print("socket failed\n", .{});
        return;
    }
    var addr: c.struct_sockaddr_in = std.mem.zeroes(c.struct_sockaddr_in);
    addr.sin_family = c.AF_INET;
    addr.sin_port = std.math.cast(u16, 8768) orelse 0;
    addr.sin_addr.s_addr = c.htonl(0x7f000001);
    if (c.bind(listen_fd, @ptrCast(&addr), @sizeOf(c.struct_sockaddr_in)) < 0) {
        std.debug.print("bind failed\n", .{});
        return;
    }
    if (c.listen(listen_fd, 16) < 0) {
        std.debug.print("listen failed\n", .{});
        return;
    }
    std.debug.print("listening on 127.0.0.1:8768\n", .{});
    while (true) {
        const client = c.accept(listen_fd, null, null);
        if (client < 0) break;
        handle_client(client);
    }
}
