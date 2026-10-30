def ping(n):
    from b import pong

    if n > 0:
        return pong(n - 1)
    return 0
