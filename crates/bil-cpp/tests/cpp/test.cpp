// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

// The C++ interface's test: built by GCC, Clang and MSVC against the shared library, it opens
// TIFF files written here, reads them, and checks failures, warnings, the cache and threads.

#include <bil/bil.hpp>

#include <atomic>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <sstream>
#include <thread>

namespace {

int failures = 0;

#define CHECK(cond)                                                                 \
    do {                                                                            \
        if (!(cond)) {                                                              \
            std::cerr << __FILE__ << ":" << __LINE__ << ": failed: " #cond "\n";    \
            ++failures;                                                             \
        }                                                                           \
    } while (false)

void put16(std::vector<std::byte>& b, std::uint16_t v) {
    b.push_back(std::byte(v & 0xff));
    b.push_back(std::byte(v >> 8));
}

void put32(std::vector<std::byte>& b, std::uint32_t v) {
    for (int i = 0; i < 4; ++i) b.push_back(std::byte((v >> (8 * i)) & 0xff));
}

struct Entry {
    std::uint16_t tag, type;
    std::vector<std::uint32_t> values;
};

// A little-endian TIFF of 8-bit gray strips, pixel (r, c) holding r * cols + c, with SubIFD
// pointers past the file where `sub_ifds` is set.
std::vector<std::byte> tiff(std::uint32_t rows, std::uint32_t cols, std::uint32_t strip_rows,
                            bool sub_ifds) {
    const std::uint32_t strips = (rows + strip_rows - 1) / strip_rows;
    std::vector<Entry> entries = {
        {256, 3, {cols}}, {257, 3, {rows}}, {258, 3, {8}}, {259, 3, {1}}, {262, 3, {1}},
        {273, 4, {}},     {277, 3, {1}},    {278, 3, {strip_rows}},     {279, 4, {}},
    };
    if (sub_ifds) entries.push_back({330, 4, {9000000, 9000004}});
    std::vector<std::uint32_t> counts;
    for (std::uint32_t s = 0; s < strips; ++s) {
        counts.push_back(std::min(strip_rows, rows - s * strip_rows) * cols);
    }
    // Header, IFD, then arrays too long for an entry, then the pixels.
    const std::uint32_t ifd = 8;
    const std::uint32_t ifd_end = ifd + 2 + 12 * static_cast<std::uint32_t>(entries.size()) + 4;
    std::uint32_t extra = ifd_end;
    std::vector<std::uint32_t> offsets;
    std::uint32_t pixels = ifd_end + 4 * strips * 2 + 8 * 2;
    for (std::uint32_t s = 0; s < strips; ++s) {
        offsets.push_back(pixels);
        pixels += counts[s];
    }
    entries[5].values = offsets;
    entries[8].values = counts;
    std::vector<std::byte> b;
    b.push_back(std::byte('I'));
    b.push_back(std::byte('I'));
    put16(b, 42);
    put32(b, ifd);
    put16(b, static_cast<std::uint16_t>(entries.size()));
    std::vector<std::byte> arrays;
    for (const auto& e : entries) {
        put16(b, e.tag);
        put16(b, e.type);
        put32(b, static_cast<std::uint32_t>(e.values.size()));
        const std::size_t size = e.type == 3 ? 2 : 4;
        if (e.values.size() * size <= 4) {
            if (size == 2) {
                put16(b, static_cast<std::uint16_t>(e.values[0]));
                put16(b, e.values.size() > 1 ? static_cast<std::uint16_t>(e.values[1]) : 0);
            } else {
                put32(b, e.values[0]);
            }
        } else {
            put32(b, extra + static_cast<std::uint32_t>(arrays.size()));
            for (auto v : e.values) put32(arrays, v);
        }
    }
    put32(b, 0);
    b.insert(b.end(), arrays.begin(), arrays.end());
    b.resize(offsets[0], std::byte(0));
    for (std::uint32_t r = 0; r < rows; ++r) {
        for (std::uint32_t c = 0; c < cols; ++c) b.push_back(std::byte((r * cols + c) & 0xff));
    }
    return b;
}

bool holds(const bil::Fragment& f, std::uint32_t cols) {
    const auto& w = f.window();
    for (std::uint32_t r = 0; r < w.row_count; ++r) {
        for (std::uint32_t c = 0; c < w.col_count; ++c) {
            const auto v = f.data()[r * f.strides()[0] + c * f.strides()[1]];
            const auto want = ((w.row_start + r) * cols + w.col_start + c) & 0xff;
            if (std::to_integer<unsigned>(v) != want) return false;
        }
    }
    return true;
}

template <class F>
std::optional<bil::Error> fails(F f) {
    try {
        f();
    } catch (const bil::Error& e) {
        return e;
    }
    return std::nullopt;
}

}  // namespace

int main() {
    const std::uint32_t rows = 6, cols = 5;
    const auto file = tiff(rows, cols, 2, true);

    // Opening, and what the Image is.
    auto source = bil::OpenOptions().open_from(file);
    CHECK(source.driver() == "geotiff");
    CHECK(source.image_count() == 1);
    const auto image = source.image(0);
    const auto info = image.info();
    CHECK(info.row_count == rows && info.col_count == cols);
    CHECK(info.frame_count.kind == bil::FrameCount::Kind::Known && info.frame_count.count == 1);
    CHECK(info.representation.kind == bil::Representation::Kind::Intensity);
    CHECK(info.representation.name() == "intensity");
    CHECK(!info.representation.white_is_zero);
    CHECK(info.component_names == std::vector<std::string>{"intensity"});
    CHECK(info.pixel_type && info.pixel_type->sample_type == bil::SampleType::U8);
    CHECK(info.component_count == 1);
    CHECK(info.storage.block_size == std::make_pair(2u, 5u));
    CHECK(info.storage.blocks.empty());
    CHECK(!info.may_have_missing);
    CHECK(source.list_images().size() == 1);
    CHECK(image.list_stored_levels().empty());

    // Storage and Fragments.
    const auto storage = image.describe_storage();
    CHECK(storage.order == bil::StorageOrder::Known && storage.blocks.size() == 3);
    CHECK(storage.blocks[1].block_row == 1 && storage.blocks[1].length == 10);
    const auto whole = image.read_fragment({0, 0, rows, cols});
    CHECK(whole.data().size() == rows * cols && whole.validity().empty());
    CHECK(holds(whole, cols));
    const auto part = image.read_fragment({3, 1, 2, 3}, "level0");
    CHECK(part.window().row_start == 3 && part.data().size() == 6 && holds(part, cols));
    const auto copy = part;  // shares the pixels
    CHECK(copy.data().data() == part.data().data());

    // The cache: three strips decoded once; the second read finds its two.
    auto cache = source.describe_cache();
    CHECK(cache.misses == 3 && cache.hits == 2 && cache.fragment_count == 3);
    source.set_cache_size(1);
    CHECK(source.describe_cache().fragment_count == 1);
    source.clear_cache();
    CHECK(source.describe_cache().bytes == 0);

    // Warnings: the two SubIFDs past the file were skipped.
    auto warnings = source.take_warnings();
    CHECK(warnings.dropped == 0 && warnings.warnings.size() == 2);
    const auto& w = warnings.warnings.at(0);
    CHECK(w.code == bil::WarningCode::SubdirectorySkipped);
    CHECK(bil::name(w.code) == "subdirectory_skipped");
    CHECK(w.scope.image == std::optional<std::size_t>(0) && !w.scope.is_source());
    CHECK(w.detail == std::optional<std::uint64_t>(9000000) && w.count == 1);
    std::ostringstream text;
    text << w;
    CHECK(text.str() == "image 0: the subdirectory at offset 9000000 could not be read and was "
                        "skipped");
    CHECK(source.take_warnings().warnings.empty());
    {
        bil::OpenOptions one;
        one.max_held_warnings = 1;
        CHECK(one.open_from(file).take_warnings().dropped == 1);
    }

    // Failures.
    const std::vector<std::byte> junk(16, std::byte('x'));
    auto e = fails([&] { bil::OpenOptions().open_from(junk); });
    CHECK(e && e->kind() == bil::ErrorKind::Unrecognized);
    CHECK(e && e->tried() == (std::vector<std::string>{"nitf", "geotiff"}));
    {
        // Cut short, so its strips run past the end: the structure that locates the pixels
        // cannot be trusted.
        const std::vector<std::byte> cut(file.begin(), file.end() - 10);
        e = fails([&] { bil::OpenOptions().open_from(cut); });
        CHECK(e && e->kind() == bil::ErrorKind::Corrupt);
    }
    e = fails([] { bil::open_source("/no/such/file.tif"); });
    CHECK(e && e->kind() == bil::ErrorKind::Io);
    e = fails([&] { image.read_fragment({5, 0, 2, 5}); });
    CHECK(e && e->kind() == bil::ErrorKind::NotFound);
    e = fails([&] { image.read_fragment({0, 0, 1, 1}, "stored1"); });
    CHECK(e && e->kind() == bil::ErrorKind::NotFound);
    e = fails([&] { source.image(1); });
    CHECK(e && e->kind() == bil::ErrorKind::NotFound);
    {
        bil::OpenOptions none;
        none.max_directories = 0;
        e = fails([&] { none.open_from(file); });
        CHECK(e && e->kind() == bil::ErrorKind::Unsupported);
    }

    // Threads share one Source.
    std::atomic<int> good{0};
    std::vector<std::thread> threads;
    for (int t = 0; t < 4; ++t) {
        threads.emplace_back([&, t] {
            for (int i = 0; i < 50; ++i) {
                const auto f = image.read_fragment({static_cast<std::uint64_t>((t + i) % 5), 0, 1, cols});
                if (holds(f, cols)) ++good;
            }
        });
    }
    for (auto& t : threads) t.join();
    CHECK(good == 200);

    // Closing closes every copy, and a later call is the caller's misuse, C++'s logic_error,
    // not a bil::Error; what was read stays valid.
    const auto other = source;
    other.close();
    const auto misuse = [](auto f) {
        try {
            f();
        } catch (const bil::Error&) {
            return false;
        } catch (const std::logic_error& e) {
            return std::string_view(e.what()) == "bil: the Source is closed";
        }
        return false;
    };
    CHECK(misuse([&] { source.image_count(); }));
    CHECK(misuse([&] { image.info(); }));
    CHECK(misuse([&] { other.take_warnings(); }));
    CHECK(holds(whole, cols));

    if (failures == 0) std::cout << "C++ interface: every check passed\n";
    return failures == 0 ? 0 : 1;
}
