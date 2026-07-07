const std = @import("std");
const c = @cImport({
    @cInclude("unistd.h");
    @cInclude("sys/socket.h");
    @cInclude("netinet/in.h");
    @cInclude("arpa/inet.h");
    @cInclude("string.h");
    @cInclude("stdio.h");
    @cInclude("sys/stat.h");
});
extern fn bind_in(sockfd: c_int, addr: [*c]const c.struct_sockaddr_in, addrlen: c_int) c_int;

const LISTEN_PORT = 8768;
const HTML_PATH = "/Users/careybalboa/Documents/GitHub/archetype-mesh-benchmark/zig-backend/dashboard.html";
const OWL_PATH = "/Users/careybalboa/Documents/GitHub/archetype-mesh-benchmark/zig-backend/assets/owl.png";

fn send_all(client_fd: c_int, buf: []const u8) void {
    var left = buf.len;
    while (left > 0) {
        const sent = c.send(client_fd, buf.ptr + (buf.len - left), left, 0);
        if (sent <= 0) break;
        left -= @intCast(sent);
    }
}

fn read_file(path: [*c]const u8, out_buf: [*]u8, cap: usize) usize {
    const fd = c.fopen(path, "rb");
    if (fd == null) return 0;
    defer _ = c.fclose(fd);
    _ = c.fseek(fd, 0, 2);
    const size = @as(usize, @intCast(c.ftell(fd)));
    if (size > cap) return 0;
    _ = c.fseek(fd, 0, 0);
    const n = @as(usize, @intCast(c.fread(out_buf, 1, size, fd)));
    return n;
}

fn serve_file(client_fd: c_int, path: [*c]const u8, mime: [*c]const u8) void {
    var buf: [1024 * 1024]u8 = undefined;
    const n = read_file(path, &buf, buf.len);
    if (n == 0) {
        const hdr = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\n";
        send_all(client_fd, hdr);
        send_all(client_fd, "Not found");
        return;
    }
    const size = @as(usize, n);
    var len_buf: [32]u8 = undefined;
    var len_idx: usize = len_buf.len;
    var tmp = size;
    if (tmp == 0) { len_buf[len_idx - 1] = '0'; len_idx -= 1; }
    while (tmp > 0) { len_idx -= 1; len_buf[len_idx] = '0' + @as(u8, @intCast(tmp % 10)); tmp /= 10; }
    const len_str = len_buf[len_idx..];
    var hdr: [128]u8 = undefined;
    var idx: usize = 0;
    const prefix = "HTTP/1.1 200 OK\r\nContent-Type: ";
    @memcpy(hdr[idx..][0..prefix.len], prefix); idx += prefix.len;
    @memcpy(hdr[idx..][0..std.mem.len(mime)], mime); idx += std.mem.len(mime);
    hdr[idx] = '\r'; idx += 1; hdr[idx] = '\n'; idx += 1;
    const cl = "Content-Length: ";
    @memcpy(hdr[idx..][0..cl.len], cl); idx += cl.len;
    @memcpy(hdr[idx..][0..len_str.len], len_str); idx += len_str.len;
    const trailer = "Connection: close\r\n\r\n";
    @memcpy(hdr[idx..][0..trailer.len], trailer); idx += trailer.len;
    send_all(client_fd, hdr[0..idx]);
    send_all(client_fd, buf[0..size]);
}

fn write_temp_file(path: [*c]const u8, data: []const u8) void {
    const fd = c.fopen(path, "w");
    if (fd == null) return;
    _ = c.fwrite(data.ptr, 1, data.len, fd);
    _ = c.fclose(fd);
}

fn pipe_read(cmd: [*c]const u8, out_buf: [*]u8, cap: usize) usize {
    const pipe = c.popen(cmd, "r");
    if (pipe == null) return 0;
    defer _ = c.pclose(pipe);
    var idx: usize = 0;
    while (true) {
        const chunk = c.fread(out_buf[idx..][0..1], 1, 1, pipe.?);
        if (chunk == 0) break;
        idx += 1;
        if (idx >= cap) break;
    }
    return idx;
}

fn send_json(client_fd: c_int, body: []const u8) void {
    var len_buf: [32]u8 = undefined;
    var len_idx: usize = len_buf.len;
    var tmp = body.len;
    if (tmp == 0) { len_buf[len_idx - 1] = '0'; len_idx -= 1; }
    while (tmp > 0) { len_idx -= 1; len_buf[len_idx] = '0' + @as(u8, @intCast(tmp % 10)); tmp /= 10; }
    const len_str = len_buf[len_idx..];
    var hdr: [96]u8 = undefined;
    var idx: usize = 0;
    const prefix = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ";
    @memcpy(hdr[idx..][0..prefix.len], prefix); idx += prefix.len;
    @memcpy(hdr[idx..][0..len_str.len], len_str); idx += len_str.len;
    const trailer = "\r\nConnection: close\r\n\r\n";
    @memcpy(hdr[idx..][0..trailer.len], trailer); idx += trailer.len;
    send_all(client_fd, hdr[0..idx]);
    send_all(client_fd, body);
}

fn handle_client(client_fd: c_int) void {
    var buf: [4096]u8 = undefined;
    const req_len = c.recv(client_fd, &buf, buf.len, 0);
    if (req_len <= 0) {
        _ = c.close(client_fd);
        return;
    }
    const request = buf[0..@intCast(req_len)];
    var method: []const u8 = "";
    var path: []const u8 = "";
    var rest = request;
    if (std.mem.indexOfScalar(u8, rest, ' ')) |space| {
        method = rest[0..space];
        rest = rest[space + 1 ..];
        if (std.mem.indexOfScalar(u8, rest, ' ')) |space2| {
            path = rest[0..space2];
        }
    }

    if (std.mem.eql(u8, method, "GET")) {
        if (std.mem.eql(u8, path, "/") or std.mem.eql(u8, path, "/index.html")) {
            serve_file(client_fd, HTML_PATH, "text/html; charset=utf-8");
        } else if (std.mem.eql(u8, path, "/assets/owl.png")) {
            serve_file(client_fd, OWL_PATH, "image/png");
        } else if (std.mem.eql(u8, path, "/api/status")) {
            const hdr = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\n";
            send_all(client_fd, hdr);
            send_all(client_fd, "ok");
        } else if (std.mem.eql(u8, path, "/api/summary")) {
            const sql = "SELECT json_group_array(json_object('model',model,'provider',provider,'family',test,'verdict',verdict,'detail',detail,'date',date)) FROM legacy_matrix";
            write_temp_file("/tmp/query.sql", sql);
            var out: [65536]u8 = undefined;
            const n = pipe_read("/tmp/am-sqlite-query.py /tmp/query.sql", &out, out.len);
            send_json(client_fd, out[0..n]);
        } else if (std.mem.eql(u8, path, "/api/models")) {
            const sql = "SELECT json_group_array(json_object('key',model,'name',model,'provider',provider,'kind',test,'vision',0,'tools',0,'local_path',model)) FROM (SELECT DISTINCT model, provider, test FROM legacy_matrix) LIMIT 20";
            write_temp_file("/tmp/query.sql", sql);
            var out: [65536]u8 = undefined;
            const n = pipe_read("/tmp/am-sqlite-query.py /tmp/query.sql", &out, out.len);
            send_json(client_fd, out[0..n]);
        } else {
            const hdr = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\n";
            send_all(client_fd, hdr);
            send_all(client_fd, "Not found");
        }
    } else {
        const hdr = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 18\r\nConnection: close\r\n\r\n";
        send_all(client_fd, hdr);
        send_all(client_fd, "Method Not Allowed");
    }
    _ = c.close(client_fd);
}

fn ensure_helper() void {
    // Write the SQLite JSON helper to /tmp so it survives reboots
    const helper = "#!/usr/bin/env python3\n" ++
        "import sqlite3, json, sys\n" ++
        "db_path = \"/Users/careybalboa/Documents/GitHub/archetype-mesh-benchmark/data/archetype_mesh_benchmark.sqlite\"\n" ++
        "sql_file = sys.argv[1] if len(sys.argv) > 1 else None\n" ++
        "sql = \"\"\n" ++
        "if sql_file:\n" ++
        "    with open(sql_file, \"r\") as f:\n" ++
        "        sql = f.read().strip()\n" ++
        "if not sql:\n" ++
        "    print(\"[]\")\n" ++
        "    sys.exit(0)\n" ++
        "con = sqlite3.connect(db_path)\n" ++
        "cur = con.cursor()\n" ++
        "try:\n" ++
        "    cur.execute(sql)\n" ++
        "    rows = cur.fetchall()\n" ++
        "    print(rows[0][0] if rows and rows[0][0] else \"[]\")\n" ++
        "except Exception as e:\n" ++
        "    print(\"[]\")\n" ++
        "finally:\n" ++
        "    con.close()\n";
    write_temp_file("/tmp/am-sqlite-query.py", helper);
    _ = c.chmod("/tmp/am-sqlite-query.py", 0o755);
}

pub fn main() void {
    ensure_helper();
    const listen_fd = c.socket(c.AF_INET, c.SOCK_STREAM, 0);
    if (listen_fd < 0) {
        std.debug.print("socket failed\n", .{});
        return;
    }
    // SO_REUSEADDR to avoid bind failures after previous process exits
    var reuse: c_int = 1;
    _ = c.setsockopt(listen_fd, c.SOL_SOCKET, c.SO_REUSEADDR, &reuse, @sizeOf(c_int));
    var addr: c.struct_sockaddr_in = std.mem.zeroes(c.struct_sockaddr_in);
    addr.sin_len = @sizeOf(c.struct_sockaddr_in);
    addr.sin_family = c.AF_INET;
    addr.sin_port = c.htons(LISTEN_PORT);
    addr.sin_addr.s_addr = c.htonl(0x7f000001);
    if (bind_in(listen_fd, &addr, @sizeOf(c.struct_sockaddr_in)) != 0) {
        std.debug.print("bind failed\n", .{});
        _ = c.close(listen_fd);
        return;
    }
    if (c.listen(listen_fd, 16) < 0) {
        std.debug.print("listen failed\n", .{});
        _ = c.close(listen_fd);
        return;
    }
    std.debug.print("listening on 127.0.0.1:{d} fd={d}\n", .{LISTEN_PORT, listen_fd});
    while (true) {
        const client = c.accept(listen_fd, null, null);
        if (client < 0) continue;
        handle_client(client);
    }
}
