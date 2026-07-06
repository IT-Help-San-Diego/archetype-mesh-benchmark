const std = @import("std");
const Sha3_256 = std.crypto.sha3.Sha3_256;

pub fn main() !void {
    const payload = std.posix.getenv("PAYLOAD") orelse "hello";

    var out: [Sha3_256.digest_length]u8 = undefined;
    Sha3_256.hash(payload, &out, .{});

    std.debug.print("sha3-256: ", .{});
    for (out) |byte| {
        std.debug.print("{x:0>2}", .{byte});
    }
    std.debug.print("\n", .{});
}
