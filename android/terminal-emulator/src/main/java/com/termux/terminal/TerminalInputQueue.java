package com.termux.terminal;

import java.util.ArrayDeque;

/** A bounded input queue: the UI must never wait for a terminal to read stdin. */
final class TerminalInputQueue {
    static final int MAX_BYTES = 4 * 1024 * 1024;
    private static final int CHUNK_BYTES = 4096;
    private static final class Chunk {
        final byte[] data = new byte[CHUNK_BYTES];
        int length;
    }
    private final ArrayDeque<Chunk> chunks = new ArrayDeque<>();
    private int offset;
    private int size;
    private boolean open = true;
    private int generation;
    static final class Read {
        final int count, generation;
        Read(int count, int generation) { this.count = count; this.generation = generation; }
    }
    synchronized int generation() { return generation; }

    synchronized boolean offer(byte[] data, int start, int count) {
        if (start < 0 || count < 0 || start > data.length - count) throw new IndexOutOfBoundsException();
        if (!open || count > MAX_BYTES - size) return false;
        if (count > 0) {
            int at = start;
            int left = count;
            while (left > 0) {
                Chunk tail = chunks.peekLast();
                if (tail == null || tail.length == CHUNK_BYTES) {
                    tail = new Chunk();
                    chunks.addLast(tail);
                }
                int n = Math.min(left, CHUNK_BYTES - tail.length);
                System.arraycopy(data, at, tail.data, tail.length, n);
                tail.length += n;
                at += n;
                left -= n;
            }
            size += count;
            notifyAll();
        }
        return true;
    }

    synchronized Read read(byte[] out) {
        while (open && size == 0) {
            try { wait(); } catch (InterruptedException ignored) { }
        }
        if (!open) return null;
        int total = 0;
        while (total < out.length && size > 0) {
            Chunk first = chunks.getFirst();
            int count = Math.min(out.length - total, first.length - offset);
            System.arraycopy(first.data, offset, out, total, count);
            total += count;
            offset += count;
            size -= count;
            if (offset == first.length) { chunks.removeFirst(); offset = 0; }
        }
        return new Read(total, generation);
    }

    synchronized void clear() { chunks.clear(); offset = size = 0; generation++; }

    synchronized void close() { open = false; clear(); notifyAll(); }
}
