// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository: the exception covers code a program includes from
// this header and compiles as part of itself.

// The Blazing Imaging Library's C++ interface (specification, section 2): Rust's names and
// shapes, in the namespace `bil`. C++20, header-only, over the binary layer the shared library
// `bil` exports. That layer is BIL's own and not an interface for callers; only this header is.

#ifndef BIL_BIL_HPP
#define BIL_BIL_HPP

#include <array>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <memory>
#include <optional>
#include <ostream>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

// ------------------------------------------------------------------ the binary layer

namespace bil::detail {
// Where the layer's text goes. The sinks never throw, since an exception must not unwind
// through the library's frames; a failure to hold the text is noted and thrown afterwards.
struct Text {
    std::string text;
    bool failed = false;
};
struct Texts {
    std::vector<std::string> texts;
    bool failed = false;
};
}  // namespace bil::detail

extern "C" {
struct bil_abi_error;
struct bil_abi_source;
struct bil_abi_warnings;
struct bil_abi_storage;
struct bil_abi_fragment;
struct bil_abi_slice;
typedef void (*bil_abi_sink)(void* ctx, const unsigned char* text, std::size_t len);

struct bil_abi_open_options {
    std::size_t size;
    std::uint64_t max_directories, max_subdirectories, max_held_warnings;
};
struct bil_abi_cache_report {
    std::size_t fragment_count;
    std::uint64_t bytes;
    std::size_t max_fragments;
    std::uint64_t max_bytes, hits, misses, served_from_read_ahead;
};
struct bil_abi_warning {
    std::uint32_t code;
    std::int64_t image;
    std::uint8_t has_detail;
    std::uint64_t detail, count;
    std::size_t block_count;
};
struct bil_abi_block_key {
    std::uint32_t image, level;
    std::uint64_t frame;
    std::uint32_t block_row, block_col;
    std::uint32_t components[2];
};
struct bil_abi_pixel_type {
    std::uint32_t sample_type, component_count, significant_bits, bit_shift;
};
struct bil_abi_image_info {
    std::uint64_t row_count, col_count;
    std::uint32_t frame_kind;
    std::uint64_t frame_count;
    std::uint32_t representation;
    std::uint8_t white_is_zero;
    std::uint32_t color_model;
    double coefficients[3];
    std::uint32_t lab_encoding;
    std::uint8_t has_pixel_type;
    bil_abi_pixel_type pixel_type;
    std::uint32_t component_count;
    std::size_t name_count, relation_count;
    std::uint32_t block_size[2];
    std::uint32_t interleave, order;
    std::uint8_t may_have_missing;
};
struct bil_abi_relation {
    std::size_t image;
    std::uint32_t kind;
    std::uint32_t reduction[2];
    double position[2];
};
struct bil_abi_stored_level {
    std::uint64_t rows, cols;
    double scale[2], position[2];
    std::int8_t aligned;
};
struct bil_abi_storage_head {
    std::uint32_t block_size[2];
    std::uint32_t interleave, order;
    std::size_t block_count;
};
struct bil_abi_block {
    std::uint32_t image, level;
    std::uint64_t frame;
    std::uint32_t block_row, block_col;
    std::uint32_t components[2];
    std::size_t range_count;
};
struct bil_abi_window {
    std::uint64_t row_start, col_start;
    std::uint32_t row_count, col_count;
};
struct bil_abi_slice_view {
    const char* name;
    std::size_t name_len;
    const char* label;
    std::size_t label_len;
    const char* level_name;
    std::size_t level_name_len;
    std::uint32_t status;
    double origin[2];
    double scale[2];
    bil_abi_window window;
    std::uint32_t row_count, col_count;
    const float* data;
    std::size_t data_len;
    std::ptrdiff_t strides[2];
};
struct bil_abi_fragment_view {
    bil_abi_window window;
    bil_abi_pixel_type pixel_type;
    std::size_t strides[3];
    const unsigned char* data;
    std::size_t data_len;
    const bool* validity;
    std::size_t validity_len;
};

std::uint32_t bil_abi_version(void);
std::uint32_t bil_abi_error_kind(const bil_abi_error*);
void bil_abi_error_message(const bil_abi_error*, bil_abi_sink, void*);
void bil_abi_error_tried(const bil_abi_error*, bil_abi_sink, void*);
void bil_abi_error_free(bil_abi_error*);
void bil_abi_open_options_default(bil_abi_open_options*);
bil_abi_error* bil_abi_open_path(const bil_abi_open_options*, const char*, std::size_t,
                                 const bil_abi_source**);
bil_abi_error* bil_abi_open_bytes(const bil_abi_open_options*, const std::byte*, std::size_t,
                                  const bil_abi_source**);
void bil_abi_source_release(const bil_abi_source*);
void bil_abi_source_close(const bil_abi_source*);
bil_abi_error* bil_abi_source_driver(const bil_abi_source*, bil_abi_sink, void*);
bil_abi_error* bil_abi_source_image_count(const bil_abi_source*, std::size_t*);
bil_abi_error* bil_abi_set_cache_size(const bil_abi_source*, std::size_t);
bil_abi_error* bil_abi_set_cache_bytes(const bil_abi_source*, std::uint64_t);
bil_abi_error* bil_abi_clear_cache(const bil_abi_source*);
bil_abi_error* bil_abi_describe_cache(const bil_abi_source*, bil_abi_cache_report*);
bil_abi_error* bil_abi_take_warnings(const bil_abi_source*, bil_abi_warnings**);
std::size_t bil_abi_warnings_count(const bil_abi_warnings*);
std::uint64_t bil_abi_warnings_dropped(const bil_abi_warnings*);
void bil_abi_warning_get(const bil_abi_warnings*, std::size_t, bil_abi_warning*);
void bil_abi_warning_block(const bil_abi_warnings*, std::size_t, std::size_t, bil_abi_block_key*);
void bil_abi_warning_message(const bil_abi_warnings*, std::size_t, bil_abi_sink, void*);
void bil_abi_warnings_free(bil_abi_warnings*);
bil_abi_error* bil_abi_image_info_get(const bil_abi_source*, std::size_t, bil_abi_image_info*);
bil_abi_error* bil_abi_image_component_names(const bil_abi_source*, std::size_t, bil_abi_sink,
                                             void*);
bil_abi_error* bil_abi_image_relation(const bil_abi_source*, std::size_t, std::size_t,
                                      bil_abi_relation*);
bil_abi_error* bil_abi_stored_level_count(const bil_abi_source*, std::size_t, std::size_t*);
bil_abi_error* bil_abi_stored_level_get(const bil_abi_source*, std::size_t, std::size_t,
                                    bil_abi_stored_level*, bil_abi_sink, void*);
bil_abi_error* bil_abi_describe_storage(const bil_abi_source*, std::size_t, const char*,
                                        std::size_t, bil_abi_storage**);
void bil_abi_storage_head_get(const bil_abi_storage*, bil_abi_storage_head*);
void bil_abi_storage_block(const bil_abi_storage*, std::size_t, bil_abi_block*);
void bil_abi_storage_block_range(const bil_abi_storage*, std::size_t, std::size_t, std::uint64_t*);
void bil_abi_storage_free(bil_abi_storage*);
bil_abi_error* bil_abi_read_fragment(const bil_abi_source*, std::size_t, const bil_abi_window*,
                                     const char*, std::size_t, bil_abi_fragment**);
void bil_abi_fragment_view_get(const bil_abi_fragment*, bil_abi_fragment_view*);
void bil_abi_fragment_free(bil_abi_fragment*);
bil_abi_error* bil_abi_read_slice(const bil_abi_source*, std::size_t, const char*, std::size_t,
                                  const bil_abi_window*, const char*, std::size_t,
                                  bil_abi_slice**);
void bil_abi_slice_view_get(const bil_abi_slice*, bil_abi_slice_view*);
void bil_abi_slice_free(bil_abi_slice*);

// Appends to the `bil::detail::Text` at `ctx`.
inline void bil_abi_cpp_append(void* ctx, const unsigned char* text, std::size_t len) noexcept {
    auto* t = static_cast<bil::detail::Text*>(ctx);
    try {
        t->text.append(reinterpret_cast<const char*>(text), len);
    } catch (...) {
        t->failed = true;
    }
}
// Appends one string to the `bil::detail::Texts` at `ctx`.
inline void bil_abi_cpp_push(void* ctx, const unsigned char* text, std::size_t len) noexcept {
    auto* t = static_cast<bil::detail::Texts*>(ctx);
    try {
        t->texts.emplace_back(reinterpret_cast<const char*>(text), len);
    } catch (...) {
        t->failed = true;
    }
}
}

namespace bil {

/// The version of the binary layer this header was written for.
inline constexpr std::uint32_t abi_version = 1;

// ------------------------------------------------------------------ failures

/// The kinds of `Error`, as Rust's `bil::Error` names its variants (section 3.3).
enum class ErrorKind { Unrecognized = 1, Corrupt, Unsupported, NotFound, Io, Internal };

/// What every failed call throws.
class Error : public std::runtime_error {
public:
    Error(ErrorKind kind, std::string message, std::vector<std::string> tried = {})
        : std::runtime_error(std::move(message)), kind_(kind), tried_(std::move(tried)) {}
    ErrorKind kind() const noexcept { return kind_; }
    /// For `Unrecognized`, the drivers tried, in the order tried.
    const std::vector<std::string>& tried() const noexcept { return tried_; }

private:
    ErrorKind kind_;
    std::vector<std::string> tried_;
};

namespace detail {

inline std::string take(Text& t) {
    if (t.failed) throw std::bad_alloc();
    return std::move(t.text);
}

inline std::vector<std::string> take(Texts& t) {
    if (t.failed) throw std::bad_alloc();
    return std::move(t.texts);
}

inline void check(bil_abi_error* e) {
    if (e == nullptr) return;
    Text message;
    Texts tried;
    bil_abi_error_message(e, bil_abi_cpp_append, &message);
    bil_abi_error_tried(e, bil_abi_cpp_push, &tried);
    const auto kind = bil_abi_error_kind(e);
    bil_abi_error_free(e);
    // A call on a closed Source is the caller's misuse, not a kind of BIL's (section 3.3).
    if (kind == 0) throw std::logic_error("bil: " + take(message));
    throw Error(static_cast<ErrorKind>(kind), take(message), take(tried));
}

/// The library must be the one this header was written for; checked once.
inline void check_version() {
    static const bool ok = bil_abi_version() == abi_version;
    if (!ok) {
        throw Error(ErrorKind::Unsupported,
                    "the bil shared library's binary layer is version " +
                        std::to_string(bil_abi_version()) + ", and this header needs version " +
                        std::to_string(abi_version));
    }
}

template <class T, void (*Free)(T*)>
struct Deleter {
    void operator()(T* p) const noexcept { Free(p); }
};

}  // namespace detail

// ------------------------------------------------------------------ plain types

enum class SampleType {
    U8, I8, U16, I16, U32, I32, U64, I64, F16, F32, F64,
    ComplexI8, ComplexI16, ComplexF32, ComplexF64,
};

/// A pixel's type: `component_count` samples of one sample type (section 4).
struct PixelType {
    SampleType sample_type = SampleType::U8;
    std::uint32_t component_count = 0;
    std::uint32_t significant_bits = 0;
    std::uint32_t bit_shift = 0;
};

/// A rectangle of an Image at full resolution (section 3).
struct Window {
    std::uint64_t row_start = 0;
    std::uint64_t col_start = 0;
    std::uint32_t row_count = 0;
    std::uint32_t col_count = 0;
};

enum class Interleave { Pixel, Line, Band };
enum class StorageOrder { Known, Unknown };

/// One run of a block's bytes in the file.
struct ByteRange {
    std::uint64_t offset = 0;
    std::uint64_t length = 0;
};

/// One block, as `describe_storage` lists it (section 4): one byte range, or several where its
/// data lies in parts, as a JPEG 2000 tile's tile-parts may (M74).
struct Block {
    std::uint32_t image = 0;
    std::uint32_t level = 0;
    std::uint64_t frame = 0;
    std::uint32_t block_row = 0;
    std::uint32_t block_col = 0;
    /// The components it holds: the first, and one past the last.
    std::pair<std::uint32_t, std::uint32_t> components;
    /// Its byte ranges, in the order the block's data is read.
    std::vector<ByteRange> ranges;

    /// Where its first byte range starts, by which blocks are listed in file order.
    std::uint64_t offset() const { return ranges.empty() ? 0 : ranges.front().offset; }
    /// Its bytes in the file over all its ranges.
    std::uint64_t length() const {
        std::uint64_t n = 0;
        for (const ByteRange& r : ranges) n += r.length;
        return n;
    }
};

/// How a level's blocks lie in the file (section 4).
struct Storage {
    std::pair<std::uint32_t, std::uint32_t> block_size;
    Interleave interleave = Interleave::Pixel;
    StorageOrder order = StorageOrder::Unknown;
    std::vector<Block> blocks;
};

/// What identifies one block in the cache and in a warning.
struct BlockKey {
    std::uint32_t image = 0;
    std::uint32_t level = 0;
    std::uint64_t frame = 0;
    std::uint32_t block_row = 0;
    std::uint32_t block_col = 0;
    std::pair<std::uint32_t, std::uint32_t> components;
};

/// The Source, or one of its Images: Rust's `Scope::Source` and `Scope::Image(i)`.
struct Scope {
    std::optional<std::size_t> image;
    bool is_source() const noexcept { return !image.has_value(); }
};

/// How many frames an Image holds (section 3.2).
struct FrameCount {
    enum class Kind { Known, AtLeast, Unknown } kind = Kind::Known;
    std::uint64_t count = 1;
};

enum class LabEncoding { Cie, Icc, Itu };

/// A color representation's model (section 3, "Representations").
struct ColorModel {
    enum class Kind { Rgb, YCbCr, Lab, Cmyk } kind = Kind::Rgb;
    /// YCbCr's luma weights of red, green and blue.
    std::array<double, 3> coefficients{};
    LabEncoding encoding = LabEncoding::Cie;
};

/// What an Image's samples are (section 3, "Representations").
struct Representation {
    enum class Kind { Intensity, Color, Palette, Complex, Spectral, Vector } kind = Kind::Vector;
    bool white_is_zero = false;
    ColorModel color;
    /// Its snake_case name.
    std::string_view name() const noexcept {
        constexpr std::string_view names[] = {"intensity", "color", "palette",
                                              "complex", "spectral", "vector"};
        return names[static_cast<int>(kind)];
    }
};

/// How one Image relates to another (section 3, "Related Images").
struct Relation {
    std::size_t image = 0;
    enum class Kind { SameGrid, ScaledGrid } kind = Kind::SameGrid;
    std::pair<std::uint32_t, std::uint32_t> reduction{1, 1};
    std::pair<double, double> position{0, 0};
};

/// One Image, described the same way by every driver (section 3).
struct ImageInfo {
    std::uint64_t row_count = 0;
    std::uint64_t col_count = 0;
    FrameCount frame_count;
    Representation representation;
    std::vector<std::string> component_names;
    /// None where BIL's types do not hold the samples; reads then fail.
    std::optional<PixelType> pixel_type;
    std::uint32_t component_count = 0;
    std::vector<Relation> relations;
    /// Block size, interleave and order; no blocks listed.
    Storage storage;
    bool may_have_missing = false;
};

/// Reduced-resolution data a file holds of an Image (section 5).
struct StoredLevel {
    std::string name;
    std::uint64_t rows = 0;
    std::uint64_t cols = 0;
    std::pair<double, double> scale;
    std::pair<double, double> position;
    /// None where the file does not say.
    std::optional<bool> aligned;
};

/// What `describe_cache` reports (section 4). Counts are in blocks.
struct CacheReport {
    std::size_t fragment_count = 0;
    std::uint64_t bytes = 0;
    std::size_t max_fragments = 0;
    std::uint64_t max_bytes = 0;
    std::uint64_t hits = 0;
    std::uint64_t misses = 0;
    std::uint64_t served_from_read_ahead = 0;
};

// ------------------------------------------------------------------ warnings

enum class WarningCode {
    CacheCeilingExceeded = 1,
    SubdirectorySkipped,
    CorruptBlockMissing,
    BadlyPacked,
};

/// A warning code's snake_case name.
inline std::string_view name(WarningCode code) noexcept {
    constexpr std::string_view names[] = {"cache_ceiling_exceeded", "subdirectory_skipped",
                                          "corrupt_block_missing", "badly_packed"};
    return names[static_cast<int>(code) - 1];
}

/// One warning, however often it occurred (section 3.3). Its message is built when streamed.
class Warning {
public:
    WarningCode code = WarningCode::CacheCeilingExceeded;
    Scope scope;
    std::optional<std::uint64_t> detail;
    std::uint64_t count = 0;
    /// The first blocks it occurred on, at most four.
    std::vector<BlockKey> blocks;

    /// The message, built now.
    std::string message() const {
        if (!set_) return {};
        detail::Text text;
        bil_abi_warning_message(set_.get(), index_, bil_abi_cpp_append, &text);
        return detail::take(text);
    }
    friend std::ostream& operator<<(std::ostream& out, const Warning& w) {
        return out << w.message();
    }

private:
    friend class Source;
    std::shared_ptr<const bil_abi_warnings> set_;
    std::size_t index_ = 0;
};

/// What `take_warnings` returns: the warnings held, in the order first met, and how many
/// distinct ones were dropped past `max_held_warnings`.
struct Warnings {
    std::vector<Warning> warnings;
    std::uint64_t dropped = 0;
};

// ------------------------------------------------------------------ Fragments

/// The native pixels of a Window (section 4). Copies share the pixels, which stay valid while
/// any copy lives.
class Fragment {
public:
    const Window& window() const noexcept { return window_; }
    const PixelType& pixel_type() const noexcept { return pixel_type_; }
    /// Bytes from one row, one column and one component to the next.
    const std::array<std::size_t, 3>& strides() const noexcept { return strides_; }
    /// The samples, in native byte order, pixel-interleaved.
    std::span<const std::byte> data() const noexcept { return data_; }
    /// One entry a pixel, row by row: false where it was never recorded. Empty where every
    /// pixel was.
    std::span<const bool> validity() const noexcept { return validity_; }

private:
    friend class Image;
    explicit Fragment(bil_abi_fragment* f)
        : handle_(f, detail::Deleter<bil_abi_fragment, bil_abi_fragment_free>{}) {
        bil_abi_fragment_view v{};
        bil_abi_fragment_view_get(f, &v);
        window_ = {v.window.row_start, v.window.col_start, v.window.row_count, v.window.col_count};
        pixel_type_ = {static_cast<SampleType>(v.pixel_type.sample_type),
                       v.pixel_type.component_count, v.pixel_type.significant_bits,
                       v.pixel_type.bit_shift};
        strides_ = {v.strides[0], v.strides[1], v.strides[2]};
        data_ = {reinterpret_cast<const std::byte*>(v.data), v.data_len};
        if (v.validity != nullptr) validity_ = {v.validity, v.validity_len};
    }
    std::shared_ptr<bil_abi_fragment> handle_;
    Window window_;
    PixelType pixel_type_;
    std::array<std::size_t, 3> strides_{};
    std::span<const std::byte> data_;
    std::span<const bool> validity_;
};

// ------------------------------------------------------------------ Slices

/// How much of what a read asked for it delivers (section 3.3).
enum class Status { Complete, Partial, Missing, NotConverged, Ambiguous };

/// Float32 values of one Slice at one Level (section 5): at grid point (i, j), `data()[i ×
/// col_count() + j]`, for the Image position `origin()` + (i, j) × `scale()`. Copies share the
/// values, which stay valid while any copy lives.
class Slice {
public:
    /// The Slice's own name, such as `intensity`.
    const std::string& name() const noexcept { return name_; }
    const std::string& label() const noexcept { return label_; }
    Status status() const noexcept { return status_; }
    /// The Level as asked: `level0`, `level3`, `stored2`, `stored2.level1`.
    const std::string& level_name() const noexcept { return level_name_; }
    /// The Image position of the first value, in full-resolution pixels: row, column.
    const std::array<double, 2>& origin() const noexcept { return origin_; }
    /// Full-resolution pixels between values: rows, columns.
    const std::array<double, 2>& scale() const noexcept { return scale_; }
    /// The Window asked for, at full resolution.
    const Window& window() const noexcept { return window_; }
    std::uint32_t row_count() const noexcept { return row_count_; }
    std::uint32_t col_count() const noexcept { return col_count_; }
    std::span<const float> data() const noexcept { return data_; }
    /// Bytes from one row and one column to the next.
    const std::array<std::ptrdiff_t, 2>& strides() const noexcept { return strides_; }

private:
    friend class Image;
    explicit Slice(bil_abi_slice* s)
        : handle_(s, detail::Deleter<bil_abi_slice, bil_abi_slice_free>{}) {
        bil_abi_slice_view v{};
        bil_abi_slice_view_get(s, &v);
        name_.assign(v.name, v.name_len);
        label_.assign(v.label, v.label_len);
        level_name_.assign(v.level_name, v.level_name_len);
        status_ = static_cast<Status>(v.status);
        origin_ = {v.origin[0], v.origin[1]};
        scale_ = {v.scale[0], v.scale[1]};
        window_ = {v.window.row_start, v.window.col_start, v.window.row_count, v.window.col_count};
        row_count_ = v.row_count;
        col_count_ = v.col_count;
        data_ = {v.data, v.data_len};
        strides_ = {v.strides[0], v.strides[1]};
    }
    std::shared_ptr<bil_abi_slice> handle_;
    std::string name_, label_, level_name_;
    Status status_ = Status::Complete;
    std::array<double, 2> origin_{}, scale_{};
    Window window_;
    std::uint32_t row_count_ = 0, col_count_ = 0;
    std::span<const float> data_;
    std::array<std::ptrdiff_t, 2> strides_{};
};

// ------------------------------------------------------------------ Sources and Images

class Source;

/// One Image of a Source (section 3). Holds its Source, so it may outlive the object it came
/// from.
class Image {
public:
    std::size_t index() const noexcept { return index_; }
    ImageInfo info() const;
    std::vector<StoredLevel> list_stored_levels() const;
    Storage describe_storage(std::string_view level = "level0") const;
    Fragment read_fragment(const Window& window, std::string_view level = "level0") const;
    /// One Slice at one Level: `pixel.raw` for now, by its full name (M75).
    Slice read_slice(std::string_view slice_name, const Window& window,
                     std::string_view level = "level0") const;

private:
    friend class Source;
    Image(std::shared_ptr<const bil_abi_source> source, std::size_t index)
        : source_(std::move(source)), index_(index) {}
    std::shared_ptr<const bil_abi_source> source_;
    std::size_t index_;
};

/// One file opened by a driver (section 3). Copies share one Source, which closes when the
/// last copy goes, or for every copy at `close()`. May be shared between threads.
class Source {
public:
    std::string driver() const {
        detail::Text out;
        detail::check(bil_abi_source_driver(h(), bil_abi_cpp_append, &out));
        return detail::take(out);
    }
    std::size_t image_count() const {
        std::size_t n = 0;
        detail::check(bil_abi_source_image_count(h(), &n));
        return n;
    }
    Image image(std::size_t index) const {
        if (index >= image_count()) {
            throw Error(ErrorKind::NotFound, "not found: image " + std::to_string(index));
        }
        return Image(handle_, index);
    }
    std::vector<ImageInfo> list_images() const {
        std::vector<ImageInfo> out;
        for (std::size_t i = 0, n = image_count(); i < n; ++i) out.push_back(image(i).info());
        return out;
    }
    void set_cache_size(std::size_t fragment_count) const {
        detail::check(bil_abi_set_cache_size(h(), fragment_count));
    }
    void set_cache_bytes(std::uint64_t byte_count) const {
        detail::check(bil_abi_set_cache_bytes(h(), byte_count));
    }
    CacheReport describe_cache() const {
        bil_abi_cache_report r{};
        detail::check(bil_abi_describe_cache(h(), &r));
        return {r.fragment_count, r.bytes, r.max_fragments, r.max_bytes,
                r.hits, r.misses, r.served_from_read_ahead};
    }
    void clear_cache() const { detail::check(bil_abi_clear_cache(h())); }
    /// The warnings recorded since the last call; empties the store (section 3.3).
    Warnings take_warnings() const {
        bil_abi_warnings* raw = nullptr;
        detail::check(bil_abi_take_warnings(h(), &raw));
        std::shared_ptr<const bil_abi_warnings> set(raw, [](const bil_abi_warnings* p) {
            bil_abi_warnings_free(const_cast<bil_abi_warnings*>(p));
        });
        Warnings out;
        out.dropped = bil_abi_warnings_dropped(raw);
        for (std::size_t i = 0, n = bil_abi_warnings_count(raw); i < n; ++i) {
            bil_abi_warning a{};
            bil_abi_warning_get(raw, i, &a);
            Warning w;
            w.code = static_cast<WarningCode>(a.code);
            if (a.image >= 0) w.scope.image = static_cast<std::size_t>(a.image);
            if (a.has_detail) w.detail = a.detail;
            w.count = a.count;
            for (std::size_t k = 0; k < a.block_count; ++k) {
                bil_abi_block_key b{};
                bil_abi_warning_block(raw, i, k, &b);
                w.blocks.push_back({b.image, b.level, b.frame, b.block_row, b.block_col,
                                    {b.components[0], b.components[1]}});
            }
            w.set_ = set;
            w.index_ = i;
            out.warnings.push_back(std::move(w));
        }
        return out;
    }
    /// Frees what the Source holds; a later call through any copy throws `std::logic_error`.
    void close() const { bil_abi_source_close(h()); }

private:
    friend struct OpenOptions;
    explicit Source(const bil_abi_source* h)
        : handle_(h, [](const bil_abi_source* p) { bil_abi_source_release(p); }) {}
    const bil_abi_source* h() const noexcept { return handle_.get(); }
    std::shared_ptr<const bil_abi_source> handle_;
};

/// What `open_source` is given (section 3.4). Constructed with the library's own defaults.
struct OpenOptions {
    std::uint64_t max_directories;
    std::uint64_t max_subdirectories;
    std::uint64_t max_held_warnings;

    OpenOptions() {
        detail::check_version();
        bil_abi_open_options o{sizeof(bil_abi_open_options), 0, 0, 0};
        bil_abi_open_options_default(&o);
        max_directories = o.max_directories;
        max_subdirectories = o.max_subdirectories;
        max_held_warnings = o.max_held_warnings;
    }
    /// Opens a file with these options.
    Source open(const std::filesystem::path& path) const {
        const std::u8string text = path.u8string();
        const bil_abi_source* h = nullptr;
        const auto o = abi();
        detail::check(bil_abi_open_path(&o, reinterpret_cast<const char*>(text.data()),
                                        text.size(), &h));
        return Source(h);
    }
    /// Opens a copy of bytes held in memory with these options.
    Source open_from(std::span<const std::byte> bytes) const {
        const bil_abi_source* h = nullptr;
        const auto o = abi();
        detail::check(bil_abi_open_bytes(&o, bytes.data(), bytes.size(), &h));
        return Source(h);
    }

private:
    bil_abi_open_options abi() const {
        return {sizeof(bil_abi_open_options), max_directories, max_subdirectories,
                max_held_warnings};
    }
};

/// Opens a file with the default options (section 2).
inline Source open_source(const std::filesystem::path& path) { return OpenOptions().open(path); }

// ------------------------------------------------------------------ Image calls

namespace detail {
inline Storage storage(bil_abi_storage_head h) {
    Storage s;
    s.block_size = {h.block_size[0], h.block_size[1]};
    s.interleave = static_cast<Interleave>(h.interleave);
    s.order = static_cast<StorageOrder>(h.order);
    return s;
}
}  // namespace detail

inline ImageInfo Image::info() const {
    bil_abi_image_info a{};
    detail::check(bil_abi_image_info_get(source_.get(), index_, &a));
    ImageInfo info;
    info.row_count = a.row_count;
    info.col_count = a.col_count;
    info.frame_count = {static_cast<FrameCount::Kind>(a.frame_kind), a.frame_count};
    info.representation.kind = static_cast<Representation::Kind>(a.representation);
    info.representation.white_is_zero = a.white_is_zero != 0;
    info.representation.color.kind = static_cast<ColorModel::Kind>(a.color_model);
    info.representation.color.coefficients = {a.coefficients[0], a.coefficients[1],
                                              a.coefficients[2]};
    info.representation.color.encoding = static_cast<LabEncoding>(a.lab_encoding);
    detail::Texts names;
    detail::check(bil_abi_image_component_names(source_.get(), index_, bil_abi_cpp_push, &names));
    info.component_names = detail::take(names);
    if (a.has_pixel_type) {
        info.pixel_type = PixelType{static_cast<SampleType>(a.pixel_type.sample_type),
                                    a.pixel_type.component_count, a.pixel_type.significant_bits,
                                    a.pixel_type.bit_shift};
    }
    info.component_count = a.component_count;
    for (std::size_t k = 0; k < a.relation_count; ++k) {
        bil_abi_relation r{};
        detail::check(bil_abi_image_relation(source_.get(), index_, k, &r));
        info.relations.push_back({r.image, static_cast<Relation::Kind>(r.kind),
                                  {r.reduction[0], r.reduction[1]},
                                  {r.position[0], r.position[1]}});
    }
    info.storage = detail::storage({{a.block_size[0], a.block_size[1]}, a.interleave, a.order, 0});
    info.may_have_missing = a.may_have_missing != 0;
    return info;
}

inline std::vector<StoredLevel> Image::list_stored_levels() const {
    std::size_t n = 0;
    detail::check(bil_abi_stored_level_count(source_.get(), index_, &n));
    std::vector<StoredLevel> out;
    for (std::size_t k = 0; k < n; ++k) {
        bil_abi_stored_level l{};
        StoredLevel s;
        detail::Text name;
        detail::check(
            bil_abi_stored_level_get(source_.get(), index_, k, &l, bil_abi_cpp_append, &name));
        s.name = detail::take(name);
        s.rows = l.rows;
        s.cols = l.cols;
        s.scale = {l.scale[0], l.scale[1]};
        s.position = {l.position[0], l.position[1]};
        if (l.aligned >= 0) s.aligned = l.aligned == 1;
        out.push_back(std::move(s));
    }
    return out;
}

inline Storage Image::describe_storage(std::string_view level) const {
    bil_abi_storage* raw = nullptr;
    detail::check(
        bil_abi_describe_storage(source_.get(), index_, level.data(), level.size(), &raw));
    std::unique_ptr<bil_abi_storage, detail::Deleter<bil_abi_storage, bil_abi_storage_free>> hold(
        raw);
    bil_abi_storage_head head{};
    bil_abi_storage_head_get(raw, &head);
    Storage s = detail::storage(head);
    for (std::size_t k = 0; k < head.block_count; ++k) {
        bil_abi_block b{};
        bil_abi_storage_block(raw, k, &b);
        Block block{b.image, b.level, b.frame, b.block_row, b.block_col,
                    {b.components[0], b.components[1]}, {}};
        for (std::size_t j = 0; j < b.range_count; ++j) {
            std::uint64_t r[2] = {0, 0};
            bil_abi_storage_block_range(raw, k, j, r);
            block.ranges.push_back({r[0], r[1]});
        }
        s.blocks.push_back(std::move(block));
    }
    return s;
}

inline Fragment Image::read_fragment(const Window& window, std::string_view level) const {
    const bil_abi_window w{window.row_start, window.col_start, window.row_count, window.col_count};
    bil_abi_fragment* raw = nullptr;
    detail::check(
        bil_abi_read_fragment(source_.get(), index_, &w, level.data(), level.size(), &raw));
    return Fragment(raw);
}

inline Slice Image::read_slice(std::string_view slice_name, const Window& window,
                               std::string_view level) const {
    const bil_abi_window w{window.row_start, window.col_start, window.row_count, window.col_count};
    bil_abi_slice* raw = nullptr;
    detail::check(bil_abi_read_slice(source_.get(), index_, slice_name.data(), slice_name.size(),
                                     &w, level.data(), level.size(), &raw));
    return Slice(raw);
}

}  // namespace bil

#endif
