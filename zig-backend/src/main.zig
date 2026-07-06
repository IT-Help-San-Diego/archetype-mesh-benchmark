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
    addr.sin_len = @sizeOf(c.struct_sockaddr_in);
    addr.sin_family = c.AF_INET;
    addr.sin_port = c.htons(8768);
    addr.sin_addr.s_addr = c.htonl(0x7f000001);

    if (c.bind(listen_fd, @ptrCast(&addr), @sizeOf(c.struct_sockaddr_in)) != 0) {
        std.debug.print("bind failed\n", .{});
        _ = c.close(listen_fd);
        return;
    }

    if (c.listen(listen_fd, 16) < 0) {
        std.debug.print("listen failed\n", .{});
        _ = c.close(listen_fd);
        return;
    }

    std.debug.print("listening on 127.0.0.1:8768 fd={d}\n", .{listen_fd});

    var i: usize = 0;
    while (i < 3) : (i += 1) {
        std.debug.print("loop iteration {d}\n", .{i + 1});
        const client = c.accept(listen_fd, null, null);
        std.debug.print("accept returned {d}\n", .{client});
        if (client < 0) {
            std.debug.print("accept negative, breaking\n", .{});
            break;
        }
        std.debug.print("handling client {d}\n", .{client});
        handle_client(client);
    }

    _ = c.close(listen_fd);
    std.debug.print("exited loop cleanly\n", .{});
}
