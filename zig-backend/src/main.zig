const std = @import("std");
const c = @cImport({
    @cInclude("unistd.h");
    @cInclude("sys/socket.h");
    @cInclude("netinet/in.h");
    @cInclude("arpa/inet.h");
    @cInclude("string.h");
});

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
}
