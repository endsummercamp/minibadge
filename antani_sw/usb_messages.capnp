@0x9966d27e7566db0b;

struct BadgeBound {
  union {
    setFrameBuffer @0 :SetFrameBuffer;
    setSolidColor @1 :RGB8;
    null @2 :Void;
  }
}

struct SetFrameBuffer {
  pixels @0 :List(RGB8);
}

struct RGB8 {
  r @0 :UInt8;
  g @1 :UInt8;
  b @2 :UInt8;
}

