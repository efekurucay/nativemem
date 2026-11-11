"""Local static fixture server for parallel browser tests."""

from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import socket


class Server(ThreadingHTTPServer):
    # Each browser context opens its own connection pool during simultaneous page loads.
    request_queue_size = socket.SOMAXCONN


with Server(("127.0.0.1", 4173), SimpleHTTPRequestHandler) as server:
    server.serve_forever()
