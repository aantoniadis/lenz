"""Independent Python stdlib/libzstd archive fixtures; run from any directory."""
from pathlib import Path
import bz2, ctypes, ctypes.util, io, lzma, struct, tarfile, zipfile, zlib

out = Path(__file__).parent
payload = b'\xef\xbb\xbfalpha\r\nbeta\x00\xff\n'
with zipfile.ZipFile(out / 'python.zip', 'w', compression=zipfile.ZIP_DEFLATED) as z:
    z.comment = b'archive comment\x00\xff'
    for name, mode, data in [('dir/', 0o40755, b''), ('dir/plain.txt', 0o100755, payload), ('link', 0o120777, b'dir/plain.txt'), ('untouched.bin', 0o100640, bytes(range(256))*8)]:
        info = zipfile.ZipInfo(name, (2024, 2, 3, 4, 5, 6))
        info.create_system = 3
        info.external_attr = mode << 16
        info.comment = b'entry-note'
        info.extra = struct.pack('<HH', 0xcafe, 4) + b'ABCD'
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, data)

with zipfile.ZipFile(out / 'python-padding.zip', 'w') as z:
    info = zipfile.ZipInfo('file')
    info.extra = b'\0'  # Optional field padding accepted by Python and the locked ZIP reader.
    z.writestr(info, b'padding fixture')

stream = io.BytesIO()
with tarfile.open(fileobj=stream, mode='w', format=tarfile.PAX_FORMAT) as t:
    for name, mode, data in [('dir/plain.txt', 0o640, payload), ('untouched.bin', 0o751, bytes(range(256))*8), ('long/'+'segment/'*18+'last.txt', 0o600, b'long path\n')]:
        info = tarfile.TarInfo(name)
        info.size = len(data)
        info.mode = mode
        info.uid, info.gid, info.uname, info.gname = 123, 456, 'alice', 'editors'
        info.mtime = 1700000000
        info.pax_headers = {'mtime': '1700000000.25', 'SCHILY.xattr.user.note': 'preserve metadata'}
        t.addfile(info, io.BytesIO(data))
    for name, kind in [('sym', tarfile.SYMTYPE), ('hard', tarfile.LNKTYPE)]:
        info = tarfile.TarInfo(name)
        info.type, info.linkname, info.mode = kind, 'dir/plain.txt', 0o777
        t.addfile(info)
raw = stream.getvalue() + b'TRAILER preserved\n'
(out / 'python.tar').write_bytes(raw)
extra = b'metadata'
header = b'\x1f\x8b\x08\x1e' + struct.pack('<I', 1700000123) + b'\x02\x03' + struct.pack('<H', len(extra)) + extra + b'original.tar\0' + b'original comment\0'
header += struct.pack('<H', zlib.crc32(header) & 0xffff)
compressor = zlib.compressobj(level=6, wbits=-15)
(out / 'python.tgz').write_bytes(header + compressor.compress(raw) + compressor.flush() + struct.pack('<II', zlib.crc32(raw), len(raw)))
(out / 'python.tbz2').write_bytes(bz2.compress(raw))
(out / 'python.txz').write_bytes(lzma.compress(raw))
zstd = ctypes.CDLL(ctypes.util.find_library('zstd'))
zstd.ZSTD_compressBound.argtypes = [ctypes.c_size_t]
zstd.ZSTD_compressBound.restype = ctypes.c_size_t
zstd.ZSTD_compress.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int]
zstd.ZSTD_compress.restype = ctypes.c_size_t
zstd.ZSTD_isError.argtypes = [ctypes.c_size_t]
zstd.ZSTD_isError.restype = ctypes.c_uint
buffer = ctypes.create_string_buffer(zstd.ZSTD_compressBound(len(raw)))
size = zstd.ZSTD_compress(buffer, len(buffer), raw, len(raw), 3)
assert not zstd.ZSTD_isError(size)
(out / 'python.tzst').write_bytes(buffer.raw[:size])
