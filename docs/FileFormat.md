# Janex File Format

## Data Types

### Basic Data Types

This document uses `u8`/`u16`/`u32`/`u64` to represent 8/16/32/64-bit unsigned integers,
uses `i8`/`i16`/`i32`/`i64` to represent 8/16/32/64-bit signed integers,
and uses `f32`/`f64` to represent 32/64-bit floating-point numbers.

### Complex Data Types

This document uses pseudo-code similar to C++ structs to represent complex data types. For example:

```c
struct MyStruct {
    u32         length;
    u8[length]  data;
};
```

Here, `length` is a 32-bit unsigned integer, and `data` is a byte array of length `length`.

`List<T>` represents a variable-length list, where `T` is the type of elements in the list.
The `T` type must define a special value to indicate the end of the list, and this value cannot appear in the list.
