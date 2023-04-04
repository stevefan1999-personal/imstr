use crate::data::Data;
use crate::error::*;
use std::borrow::{Borrow, BorrowMut};
use std::cmp::Ordering;
use std::convert::{AsRef, Infallible};
use std::fmt::{Debug, Display, Error as FmtError, Formatter, Write};
use std::hash::{Hash, Hasher};
use std::iter::{Extend, FromIterator};
use std::ops::{
    Add, AddAssign, Bound, Deref, DerefMut, Index, Range, RangeBounds, RangeFrom, RangeFull,
    RangeInclusive, RangeTo,
};
use std::str::FromStr;

#[cfg(feature = "std")]
use {
    std::borrow::Cow,
    std::ffi::OsStr,
    std::net::ToSocketAddrs,
    std::path::Path,
    std::rc::Rc,
    std::string::{String, ToString},
    std::sync::Arc,
    std::vec::Vec,
};

#[cfg(feature = "alloc")]
use {
    alloc::borrow::Cow,
    alloc::rc::Rc,
    alloc::string::{String, ToString},
    alloc::sync::Arc,
    alloc::vec::Vec,
};

#[cfg(all(feature = "alloc", test))]
use {alloc::boxed::Box, alloc::format, alloc::vec};

/// Threadsafe shared storage for string.
pub type Threadsafe = Arc<String>;

/// Non-threadsafe shared storage for string.
pub type Local = Rc<String>;

/// Cheaply cloneable and sliceable UTF-8 string type.
///
/// An `ImString` is a cheaply cloneable and sliceable UTF-8 string type,
/// designed to provide efficient operations for working with text data.
///
/// `ImString` is backed by a reference-counted shared
/// [`String`](std::string::String), which allows it to provide efficient
/// cloning and slicing operations. When an `ImString` is cloned or sliced,
/// it creates a new view into the underlying `String`, without copying the
/// text data. This makes working with large strings and substrings more
/// memory-efficient.
///
/// The `ImString` struct contains two fields:
///
/// - `string`: An [`Arc`](std::sync::Arc) wrapping a `String`, which ensures
///   that the underlying `String` data is shared and reference-counted.
///
/// - `offset`: A [`Range`](std::ops::Range) that defines the start and end
///   positions of the `ImString`'s view into the underlying `String`. The
///   `offset` must always point to a valid UTF-8 region inside the `string`.
///
/// Due to its design, `ImString` is especially suitable for use cases where
/// strings are frequently cloned or sliced, but modifications to the text data
/// are less common.
///
/// # Examples
///
/// Basic usage:
///
/// ```
/// use imstr::ImString;
///
/// // Create new ImString from a string literal
/// let string = ImString::from("hello world");
///
/// // Clone the ImString without copying the text data.
/// let string_clone = string.clone();
///
/// // Create a slice (substring) without copying the text data.
/// let string_slice = string.slice(0..5);
/// assert_eq!(string_slice, "hello");
/// ```
#[derive(Clone)]
pub struct ImString<S: Data<String>> {
    /// Underlying string
    string: S,
    /// Offset, must always point to valid UTF-8 region inside string.
    offset: Range<usize>,
}

fn slice_ptr_range(slice: &[u8]) -> Range<*const u8> {
    let start = slice.as_ptr();
    let end = unsafe { start.add(slice.len()) };
    start..end
}

fn try_slice_offset(current: &[u8], candidate: &[u8]) -> Option<Range<usize>> {
    let current_slice = slice_ptr_range(current);
    let candidate_slice = slice_ptr_range(candidate);
    let contains_start = current_slice.start <= candidate_slice.start;
    let contains_end = current_slice.end >= candidate_slice.end;
    if !contains_start || !contains_end {
        return None;
    }
    let offset_start = unsafe { candidate_slice.start.offset_from(current_slice.start) } as usize;
    let offset_end = unsafe { candidate_slice.end.offset_from(current_slice.start) } as usize;
    Some(offset_start..offset_end)
}

impl<S: Data<String>> ImString<S> {
    /// Returns a byte slice of this string's contents.
    ///
    /// The inverse of this method is [`from_utf8`](ImString::from_utf8) or
    /// [`from_utf8_lossy`](ImString::from_utf8_lossy).
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = ImString::from("hello");
    /// assert_eq!(string.as_bytes(), &[104, 101, 108, 108, 111]);
    /// ```
    pub fn as_bytes(&self) -> &[u8] {
        &self.string.get().as_bytes()[self.offset.clone()]
    }

    /// Return the backing [String](std::string::String)'s contents, in bytes.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = ImString::with_capacity(10);
    /// assert_eq!(string.capacity(), 10);
    /// ```
    pub fn capacity(&self) -> usize {
        self.string.get().capacity()
    }

    /// Create a new `ImString` instance from a standard library [`String`](std::string::String).
    ///
    /// This method will construct the `ImString` without needing to clone the `String` instance.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = String::from("hello");
    /// let string = ImString::from_std_string(string);
    /// ```
    pub fn from_std_string(string: String) -> Self {
        ImString {
            offset: 0..string.as_bytes().len(),
            string: S::new(string),
        }
    }

    /// Truncates this string, removing all contents.
    ///
    /// If this is the only reference to the string, it will clear the backing
    /// [String](std::string::String). If it is not, it only sets the offset to an empty slice.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let mut string = ImString::from("hello");
    /// assert_eq!(string, "hello");
    /// string.clear();
    /// assert_eq!(string, "");
    /// ```
    pub fn clear(&mut self) {
        unsafe {
            self.try_modify_unchecked(|string| string.clear());
        }
        self.offset = 0..0;
    }

    fn mut_str(&mut self) -> &mut str {
        if self.string.get_mut().is_none() {
            let string = self.as_str().to_string();
            self.offset = 0..string.len();
            self.string = S::new(string);
        }

        let string = self.string.get_mut().unwrap();
        return &mut string[self.offset.clone()];
    }

    unsafe fn try_modify_unchecked<F: FnOnce(&mut String)>(&mut self, f: F) -> bool {
        if let Some(string) = self.string.get_mut() {
            f(string);
            true
        } else {
            false
        }
    }

    /// Creates a new string with the given capacity.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let mut string = ImString::with_capacity(10);
    /// assert_eq!(string.capacity(), 10);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        ImString::from_std_string(String::with_capacity(capacity))
    }

    /// Returns the length of the string in bytes.
    ///
    /// This will not return the length in `char`s or graphemes.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = ImString::from("hello");
    /// assert_eq!(string.len(), "hello".len());
    /// ```
    pub fn len(&self) -> usize {
        self.offset.len()
    }

    /// Convert this string into a standard library [String](std::string::String).
    ///
    /// If this string has no other clones, it will return the `String` without needing to clone
    /// it.
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = ImString::from("hello");
    /// let string = string.into_std_string();
    /// assert_eq!(string, "hello");
    /// ```
    pub fn into_std_string(mut self) -> String {
        if self.offset.start != 0 {
            return self.as_str().to_string();
        }

        if let Some(string) = self.string.get_mut() {
            if string.len() != self.offset.end {
                string.truncate(self.offset.end);
            }

            std::mem::take(string)
        } else {
            self.as_str().to_string()
        }
    }

    /// Creates a new, empty `ImString`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = ImString::new();
    /// assert_eq!(string, "");
    /// ```
    pub fn new() -> Self {
        ImString::from_std_string(String::new())
    }

    /// Extracts a string slice containing the entire string.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = ImString::from("hello");
    /// assert_eq!(string.as_str(), "hello");
    /// ```
    pub fn as_str(&self) -> &str {
        let slice = &self.string.get().as_bytes()[self.offset.start..self.offset.end];
        unsafe { std::str::from_utf8_unchecked(slice) }
    }

    /// Converts a vector of bytes to a ImString.
    pub fn from_utf8(vec: Vec<u8>) -> Result<Self, FromUtf8Error> {
        Ok(ImString::from_std_string(String::from_utf8(vec)?))
    }

    /// Converts a slice of bytes to a string, including invalid characters.
    ///
    /// See [`String::from_utf8_lossy()`] for more details on this function.
    ///
    /// # Examples
    ///
    /// Basic usage:
    ///
    /// ```
    /// # use imstr::ImString;
    /// // some bytes, in a vector
    /// let sparkle_heart = vec![240, 159, 146, 150];
    ///
    /// let sparkle_heart = ImString::from_utf8_lossy(&sparkle_heart);
    ///
    /// assert_eq!(sparkle_heart, "💖");
    /// ```
    ///
    /// Incorrect bytes:
    ///
    /// ```
    /// # use imstr::ImString;
    /// // some invalid bytes
    /// let input = b"Hello \xF0\x90\x80World";
    /// let output = ImString::from_utf8_lossy(input);
    ///
    /// assert_eq!(output, "Hello �World");
    /// ```
    pub fn from_utf8_lossy(bytes: &[u8]) -> Self {
        let string = String::from_utf8_lossy(bytes).into_owned();
        ImString::from_std_string(string)
    }

    /// Converts a vector of bytes to a ImString.
    pub unsafe fn from_utf8_unchecked(vec: Vec<u8>) -> Self {
        ImString::from_std_string(String::from_utf8_unchecked(vec))
    }

    unsafe fn unchecked_append<F: FnOnce(String) -> String>(&mut self, f: F) {
        match self.string.get_mut() {
            Some(mut string_ref) if self.offset.start == 0 => {
                let mut string: String = std::mem::take(&mut string_ref);
                string.truncate(self.offset.end);
                *string_ref = f(string);
            }
            _ => {
                self.string = S::new(f(self.as_str().to_string()));
                self.offset.start = 0;
            }
        }

        self.offset.end = self.string.get().as_bytes().len();
    }

    /// Inserts a character into this string at the specified index.
    ///
    /// This is an *O(n)$ operation as it requires copying every element in the buffer.
    pub fn insert(&mut self, index: usize, c: char) {
        unsafe {
            self.unchecked_append(|mut string| {
                string.insert(index, c);
                string
            });
        }
    }

    /// Inserts a string into this string at the specified index.
    ///
    /// This is an *O(n)$ operation as it requires copying every element in the buffer.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let mut string = ImString::from("Hello!");
    /// string.insert_str(5, ", World");
    /// assert_eq!(string, "Hello, World!");
    /// ```
    pub fn insert_str(&mut self, index: usize, s: &str) {
        unsafe {
            self.unchecked_append(|mut string| {
                string.insert_str(index, s);
                string
            });
        }
    }

    pub fn truncate(&mut self, length: usize) {
        // actual new length
        let length = self.offset.start + length;

        // truncate backing string if possible
        if let Some(string) = self.string.get_mut() {
            string.truncate(length);
        }

        self.offset.end = self.offset.end.min(length);
    }

    pub fn push(&mut self, c: char) {
        unsafe {
            self.unchecked_append(|mut string| {
                string.push(c);
                string
            });
        }
    }

    pub fn push_str(&mut self, slice: &str) {
        unsafe {
            self.unchecked_append(|mut string| {
                string.push_str(slice);
                string
            });
        }
    }

    /// Returns `true` if this string has a length of zero, and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use imstr::ImString;
    /// let string = ImString::from("");
    /// assert_eq!(string.is_empty(), true);
    ///
    /// let string = ImString::from("hello");
    /// assert_eq!(string.is_empty(), false);
    /// ```
    pub fn is_empty(&self) -> bool {
        self.offset.is_empty()
    }

    /// Create a subslice of this string.
    ///
    /// This will panic if the specified range is invalid. Use the [try_slice](ImString::try_slice)
    /// method if you want to handle invalid ranges.
    pub fn slice(&self, range: impl RangeBounds<usize>) -> Self {
        self.try_slice(range).unwrap()
    }

    pub fn try_slice(&self, range: impl RangeBounds<usize>) -> Result<Self, SliceError> {
        let start = match range.start_bound() {
            Bound::Included(value) => *value,
            Bound::Excluded(value) => *value + 1,
            Bound::Unbounded => 0,
        };
        if start > self.offset.len() {
            return Err(SliceError::StartOutOfBounds);
        }
        let end = match range.end_bound() {
            Bound::Included(value) => *value - 1,
            Bound::Excluded(value) => *value,
            Bound::Unbounded => self.offset.len(),
        };
        if end < start {
            return Err(SliceError::EndBeforeStart);
        }
        if end > self.offset.len() {
            return Err(SliceError::EndOutOfBounds);
        }
        if !self.as_str().is_char_boundary(start) {
            return Err(SliceError::StartNotAligned);
        }
        if !self.as_str().is_char_boundary(end) {
            return Err(SliceError::EndNotAligned);
        }
        let slice = unsafe { self.slice_unchecked(range) };
        Ok(slice)
    }

    pub unsafe fn slice_unchecked(&self, range: impl RangeBounds<usize>) -> Self {
        let start = match range.start_bound() {
            Bound::Included(value) => *value,
            Bound::Excluded(value) => *value + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(value) => *value - 1,
            Bound::Excluded(value) => *value,
            Bound::Unbounded => self.offset.len(),
        };
        let offset = self.offset.start + start..self.offset.start + end;
        ImString {
            string: self.string.clone(),
            offset,
        }
    }

    pub fn try_str_ref(&self, string: &str) -> Option<Self> {
        self.try_slice_ref(string.as_bytes())
    }

    pub fn str_ref(&self, string: &str) -> Self {
        self.try_str_ref(string).unwrap()
    }

    pub fn try_slice_ref(&self, slice: &[u8]) -> Option<Self> {
        try_slice_offset(self.string.get().as_bytes(), slice).map(|range| ImString {
            offset: range,
            ..self.clone()
        })
    }

    pub fn slice_ref(&self, slice: &[u8]) -> Self {
        self.try_slice_ref(slice).unwrap()
    }

    pub fn try_split_off(&mut self, position: usize) -> Option<Self> {
        if position > self.offset.end {
            return None;
        }

        if !self.as_str().is_char_boundary(position) {
            return None;
        }

        let new = ImString {
            offset: position..self.offset.end,
            ..self.clone()
        };

        self.offset.end = position;
        Some(new)
    }

    pub fn split_off(&mut self, position: usize) -> Self {
        self.try_split_off(position).unwrap()
    }

    /// Returns a clone of the underlying reference-counted shared `String`.
    ///
    /// This method provides access to the raw `Arc<String>` that backs the `ImString`.
    /// Note that the returned `Arc<String>` may contain more data than the `ImString` itself,
    /// depending on the `ImString`'s `offset`. To access the string contents represented
    /// by the `ImString`, consider using `as_str()` instead.
    ///
    /// # Examples
    ///
    /// ```
    /// use imstr::ImString;
    /// use std::sync::Arc;
    ///
    /// let string: ImString = ImString::from("hello world");
    /// let raw_string: Arc<String> = string.raw_string();
    /// assert_eq!(&*raw_string, "hello world");
    /// ```
    pub fn raw_string(&self) -> S {
        self.string.clone()
    }

    /// Returns a clone of the `ImString`'s `offset` as a `Range<usize>`.
    ///
    /// The `offset` represents the start and end positions of the `ImString`'s view
    /// into the underlying `String`. This method is useful when you need to work with
    /// the raw offset values, for example, when creating a new `ImString` from a slice
    /// of the current one.
    ///
    /// # Examples
    ///
    /// ```
    /// use imstr::ImString;
    /// use std::ops::Range;
    ///
    /// let string: ImString = ImString::from("hello world");
    /// let raw_offset: Range<usize> = string.raw_offset();
    /// assert_eq!(raw_offset, 0..11);
    /// ```
    pub fn raw_offset(&self) -> Range<usize> {
        self.offset.clone()
    }

    /// An iterator over the lines of a string.
    ///
    /// Lines are split at line endings that are either newlines (`\n`) or sequences of a carriage
    /// return followed by a line feed (`\r\n`).
    ///
    /// Line terminators are not included in the lines returned by the iterator.
    ///
    /// The final line ending is optional. A string that ends with a final line ending will return
    /// the same lines as an otherwise identical string without a final line ending.
    ///
    /// This works the same way as [String::lines](std::string::String::lines), except that it
    /// returns ImString instances.
    pub fn lines(&self) -> Lines<'_, S> {
        ImStringIterator::new(self.string.clone(), self.as_str().lines())
    }
}

impl<S: Data<String>> Default for ImString<S> {
    fn default() -> Self {
        ImString::new()
    }
}

impl<S: Data<String>> From<&str> for ImString<S> {
    fn from(string: &str) -> Self {
        ImString::from_std_string(string.to_string())
    }
}

impl<S: Data<String>> From<char> for ImString<S> {
    fn from(c: char) -> Self {
        String::from(c).into()
    }
}

impl<S: Data<String>> From<String> for ImString<S> {
    fn from(string: String) -> Self {
        ImString::from_std_string(string)
    }
}

impl<'a, S: Data<String>> From<Cow<'a, str>> for ImString<S> {
    fn from(string: Cow<'a, str>) -> Self {
        ImString::from(string.into_owned())
    }
}

impl<S: Data<String>> From<ImString<S>> for String {
    fn from(string: ImString<S>) -> Self {
        string.into_std_string()
    }
}

impl<S: Data<String>> PartialEq<str> for ImString<S> {
    fn eq(&self, other: &str) -> bool {
        self.as_str().eq(other)
    }
}

impl<'a, S: Data<String>> PartialEq<&'a str> for ImString<S> {
    fn eq(&self, other: &&'a str) -> bool {
        self.as_str().eq(*other)
    }
}

impl<S: Data<String>> PartialEq<String> for ImString<S> {
    fn eq(&self, other: &String) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl<S: Data<String>, O: Data<String>> PartialEq<ImString<O>> for ImString<S> {
    fn eq(&self, other: &ImString<O>) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl<S: Data<String>> Eq for ImString<S> {}

impl<S: Data<String>> PartialOrd<ImString<S>> for ImString<S> {
    fn partial_cmp(&self, other: &ImString<S>) -> Option<Ordering> {
        self.as_str().partial_cmp(other.as_str())
    }
}

impl<S: Data<String>> Ord for ImString<S> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl<S: Data<String>> Debug for ImString<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        Debug::fmt(self.as_str(), f)
    }
}

impl<S: Data<String>> Display for ImString<S> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result<(), FmtError> {
        Display::fmt(self.as_str(), formatter)
    }
}

impl<S: Data<String>> FromStr for ImString<S> {
    type Err = Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ImString::from(s))
    }
}

// Delegate hash to contained str. This is important!
impl<S: Data<String>> Hash for ImString<S> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.as_str().hash(hasher)
    }
}

impl<S: Data<String>> Write for ImString<S> {
    fn write_str(&mut self, string: &str) -> Result<(), FmtError> {
        self.push_str(string);
        Ok(())
    }

    fn write_char(&mut self, c: char) -> Result<(), FmtError> {
        self.push(c);
        Ok(())
    }
}

impl<S: Data<String>> Index<Range<usize>> for ImString<S> {
    type Output = str;
    fn index(&self, index: Range<usize>) -> &str {
        &self.as_str()[index]
    }
}

impl<S: Data<String>> Index<RangeFrom<usize>> for ImString<S> {
    type Output = str;
    fn index(&self, index: RangeFrom<usize>) -> &str {
        &self.as_str()[index]
    }
}

impl<S: Data<String>> Index<RangeFull> for ImString<S> {
    type Output = str;
    fn index(&self, index: RangeFull) -> &str {
        &self.as_str()[index]
    }
}

impl<S: Data<String>> Index<RangeInclusive<usize>> for ImString<S> {
    type Output = str;
    fn index(&self, index: RangeInclusive<usize>) -> &str {
        &self.as_str()[index]
    }
}

impl<S: Data<String>> Index<RangeTo<usize>> for ImString<S> {
    type Output = str;
    fn index(&self, index: RangeTo<usize>) -> &str {
        &self.as_str()[index]
    }
}

pub type Lines<'a, S> = ImStringIterator<'a, S, std::str::Lines<'a>>;

pub struct ImStringIterator<'a, S: Data<String>, I: Iterator<Item = &'a str>> {
    string: S,
    iterator: I,
}

impl<'a, S: Data<String>, I: Iterator<Item = &'a str>> Iterator for ImStringIterator<'a, S, I> {
    type Item = ImString<S>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.iterator.next() {
            Some(slice) => {
                let offset =
                    try_slice_offset(self.string.get().as_bytes(), slice.as_bytes()).unwrap();
                Some(ImString {
                    string: self.string.clone(),
                    offset,
                })
            }
            None => None,
        }
    }
}

impl<'a, S: Data<String>, I: Iterator<Item = &'a str>> ImStringIterator<'a, S, I> {
    fn new(string: S, iterator: I) -> Self {
        ImStringIterator { string, iterator }
    }
}

impl<S: Data<String>> Deref for ImString<S> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<S: Data<String>> DerefMut for ImString<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.mut_str()
    }
}

impl<S: Data<String>> Borrow<str> for ImString<S> {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl<S: Data<String>> BorrowMut<str> for ImString<S> {
    fn borrow_mut(&mut self) -> &mut str {
        self.mut_str()
    }
}

impl<S: Data<String>> AsRef<str> for ImString<S> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(feature = "std")]
impl<S: Data<String>> AsRef<Path> for ImString<S> {
    fn as_ref(&self) -> &Path {
        self.as_str().as_ref()
    }
}

#[cfg(feature = "std")]
impl<S: Data<String>> AsRef<OsStr> for ImString<S> {
    fn as_ref(&self) -> &OsStr {
        self.as_str().as_ref()
    }
}

impl<S: Data<String>> AsRef<[u8]> for ImString<S> {
    fn as_ref(&self) -> &[u8] {
        self.as_str().as_ref()
    }
}

impl<S: Data<String>> AsMut<str> for ImString<S> {
    fn as_mut(&mut self) -> &mut str {
        self.mut_str()
    }
}

#[cfg(feature = "std")]
impl<S: Data<String>> ToSocketAddrs for ImString<S> {
    type Iter = <String as ToSocketAddrs>::Iter;
    fn to_socket_addrs(&self) -> std::io::Result<<String as ToSocketAddrs>::Iter> {
        self.as_str().to_socket_addrs()
    }
}

impl<S: Data<String>> Add<&str> for ImString<S> {
    type Output = ImString<S>;
    fn add(mut self, string: &str) -> Self::Output {
        self.push_str(string);
        self
    }
}

impl<S: Data<String>> AddAssign<&str> for ImString<S> {
    fn add_assign(&mut self, string: &str) {
        self.push_str(string);
    }
}

impl<S: Data<String>> Extend<char> for ImString<S> {
    fn extend<T: IntoIterator<Item = char>>(&mut self, iter: T) {
        unsafe {
            self.unchecked_append(|mut string| {
                string.extend(iter);
                string
            });
        }
    }
}

impl<'a, S: Data<String>> Extend<&'a char> for ImString<S> {
    fn extend<T: IntoIterator<Item = &'a char>>(&mut self, iter: T) {
        unsafe {
            self.unchecked_append(|mut string| {
                string.extend(iter);
                string
            });
        }
    }
}

impl<'a, S: Data<String>> Extend<&'a str> for ImString<S> {
    fn extend<T: IntoIterator<Item = &'a str>>(&mut self, iter: T) {
        unsafe {
            self.unchecked_append(|mut string| {
                string.extend(iter);
                string
            });
        }
    }
}

impl<S: Data<String>> FromIterator<char> for ImString<S> {
    fn from_iter<T: IntoIterator<Item = char>>(iter: T) -> Self {
        let mut string = ImString::new();
        string.extend(iter);
        string
    }
}

impl<'a, S: Data<String>> FromIterator<&'a char> for ImString<S> {
    fn from_iter<T: IntoIterator<Item = &'a char>>(iter: T) -> Self {
        let mut string = ImString::new();
        string.extend(iter);
        string
    }
}

impl<'a, S: Data<String>> FromIterator<&'a str> for ImString<S> {
    fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
        let mut string = ImString::new();
        string.extend(iter);
        string
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Cloned;

    fn test_strings<S: Data<String>>() -> Vec<ImString<S>> {
        let long = ImString::from("long string here");
        let world = ImString::from("world");
        let some = ImString::from("some");
        let multiline = ImString::from("some\nmulti\nline\nstring\nthat\nis\nlong");
        vec![
            ImString::new(),
            ImString::default(),
            ImString::from("hello"),
            ImString::from("0.0.0.0:800"),
            ImString::from("localhost:1234"),
            long.clone(),
            long.slice(4..10),
            long.slice(0..4),
            long.slice(4..4),
            long.slice(5..),
            long.slice(..),
            world.clone(),
            world.clone(),
            some.slice(4..),
            some,
            multiline.slice(5..15),
            multiline,
            ImString::from("\u{e4}\u{fc}\u{f6}\u{f8}\u{3a9}"),
            ImString::from("\u{1f600}\u{1f603}\u{1f604}"),
            ImString::from("o\u{308}u\u{308}a\u{308}"),
        ]
    }

    macro_rules! tests {
        () => {};
        (#[test] fn $name:ident <S: Data<String>>() $body:tt $($rest:tt)*) => {
            #[test]
            fn $name() {
                fn $name <S: Data<String>>() $body
                $name::<Threadsafe>();
                $name::<Local>();
                $name::<Cloned<String>>();
                $name::<Box<String>>();
            }
            tests!{$($rest)*}
        };
        (#[test] fn $name:ident <S: Data<String>>($string:ident: ImString<S>) $body:tt $($rest:tt)*) => {
            #[test]
            fn $name() {
                fn $name <S: Data<String>>() {
                    fn $name <S: Data<String>>($string: ImString<S>) $body
                    for string in test_strings::<S>().into_iter() {
                        $name(string);
                    }
                }
                $name::<Threadsafe>();
                $name::<Local>();
                $name::<Cloned<String>>();
                $name::<Box<String>>();
            }
            tests!{$($rest)*}
        }
    }

    tests! {
        #[test]
        fn test_new<S: Data<String>>() {
            let string: ImString<S> = ImString::new();
            assert_eq!(string.string.get().len(), 0);
            assert_eq!(string.offset, 0..0);
        }

        #[test]
        fn test_default<S: Data<String>>() {
            let string: ImString<S> = ImString::new();
            assert_eq!(string.string.get().len(), 0);
            assert_eq!(string.offset, 0..0);
        }

        #[test]
        fn test_with_capacity<S: Data<String>>() {
            for capacity in [10, 100, 256] {
                let string: ImString<S> = ImString::with_capacity(capacity);
                assert!(string.capacity() >= capacity);
                assert_eq!(string.string.get().len(), 0);
                assert_eq!(string.offset, 0..0);
            }
        }

        #[test]
        fn test_offset<S: Data<String>>(string: ImString<S>) {
            assert!(string.offset.start <= string.string.get().len());
            assert!(string.offset.start <= string.offset.end);
            assert!(string.offset.end <= string.string.get().len());
        }

        #[test]
        fn test_as_str<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.as_str(), &string.string.get()[string.offset.clone()]);
            assert_eq!(string.as_str().len(), string.len());
        }

        #[test]
        fn test_as_bytes<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.as_bytes(), &string.string.get().as_bytes()[string.offset.clone()]);
            assert_eq!(string.as_bytes().len(), string.len());
        }

        #[test]
        fn test_len<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.len(), string.offset.len());
            assert_eq!(string.len(), string.as_str().len());
            assert_eq!(string.len(), string.as_bytes().len());
        }

        #[test]
        fn test_clear<S: Data<String>>(string: ImString<S>) {
            let mut string = string;
            string.clear();
            assert_eq!(string.as_str(), "");
            assert_eq!(string.len(), 0);
        }

        #[test]
        fn test_debug<S: Data<String>>(string: ImString<S>) {
            let debug_string = format!("{string:?}");
            let debug_str = format!("{:?}", string.as_str());
            assert_eq!(debug_string, debug_str);
        }

        #[test]
        fn test_deref<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.deref(), string.as_str());
        }

        #[test]
        fn test_clone<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string, string.clone());
        }

        #[test]
        fn test_display<S: Data<String>>(string: ImString<S>) {
            let display_string = format!("{string}");
            let display_str = format!("{}", string.as_str());
            assert_eq!(display_string, display_str);
        }

        #[test]
        fn test_insert_start<S: Data<String>>(string: ImString<S>) {
            let mut string = string;
            let length = string.len();
            string.insert(0, 'h');
            assert_eq!(string.len(), length + 1);
            assert_eq!(string.chars().nth(0), Some('h'));
        }

        #[test]
        fn test_insert_one<S: Data<String>>(string: ImString<S>) {
            if !string.is_empty() && string.is_char_boundary(1) {
                let mut string = string;
                let length = string.len();
                string.insert(1, 'h');
                assert_eq!(string.len(), length + 1);
                assert_eq!(string.chars().nth(1), Some('h'));
            }
        }

        #[test]
        fn test_insert_end<S: Data<String>>(string: ImString<S>) {
            let mut string = string;
            let length = string.len();
            string.insert(length, 'h');
            assert_eq!(string.len(), length + 1);
            // FIXME
            //assert_eq!(string.chars().nth(length), Some('h'));
        }

        #[test]
        fn test_is_empty<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.is_empty(), string.len() == 0);
        }

        #[test]
        fn test_push<S: Data<String>>(string: ImString<S>) {
            let mut string = string;
            let mut std_string = string.as_str().to_string();
            let c = 'c';
            std_string.push(c);
            string.push(c);
            assert_eq!(string, std_string);
        }

        #[test]
        fn test_push_str<S: Data<String>>(string: ImString<S>) {
            let mut string = string;
            let mut std_string = string.as_str().to_string();
            let s = "string";
            std_string.push_str(s);
            string.push_str(s);
            assert_eq!(string, std_string);
        }

        #[test]
        fn test_slice_all<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.slice(..), string);
        }

        #[test]
        fn test_slice_start<S: Data<String>>(string: ImString<S>) {
            for end in 0..string.len() {
                if string.is_char_boundary(end) {
                    assert_eq!(string.slice(..end), string.as_str()[..end]);
                }
            }
        }

        #[test]
        fn test_slice_end<S: Data<String>>(string: ImString<S>) {
            for start in 0..string.len() {
                if string.is_char_boundary(start) {
                    assert_eq!(string.slice(start..), string.as_str()[start..]);
                }
            }
        }

        #[test]
        fn test_slice_middle<S: Data<String>>(string: ImString<S>) {
            for start in 0..string.len() {
                if string.is_char_boundary(start) {
                    for end in start..string.len() {
                        if string.is_char_boundary(end) {
                            assert_eq!(string.slice(start..end), string.as_str()[start..end]);
                        }
                    }
                }
            }
        }

        #[test]
        fn test_try_slice_all<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.try_slice(..).unwrap(), string);
        }

        #[test]
        fn test_try_slice_start<S: Data<String>>(string: ImString<S>) {
            for end in 0..string.len() {
                if string.is_char_boundary(end) {
                    assert_eq!(string.try_slice(..end).unwrap(), string.as_str()[..end]);
                } else {
                    // cannot get slice with end in middle of UTF-8 multibyte sequence.
                    assert_eq!(string.try_slice(..end), Err(SliceError::EndNotAligned));
                }
            }

            // cannot get slice with end pointing past the end of the string.
            assert_eq!(string.try_slice(..string.len()+1), Err(SliceError::EndOutOfBounds));
        }

        #[test]
        fn test_try_slice_end<S: Data<String>>(string: ImString<S>) {
            for start in 0..string.len() {
                if string.is_char_boundary(start) {
                    assert_eq!(string.try_slice(start..).unwrap(), string.as_str()[start..]);
                } else {
                    // cannot get slice with end in middle of UTF-8 multibyte sequence.
                    assert_eq!(string.try_slice(start..), Err(SliceError::StartNotAligned));
                }
            }

            // cannot get slice with end pointing past the end of the string.
            assert_eq!(string.try_slice(string.len()+1..), Err(SliceError::StartOutOfBounds));
        }

        #[test]
        fn test_add_assign<S: Data<String>>(string: ImString<S>) {
            let mut std_string = string.as_str().to_string();
            let mut string = string;
            string += "hello";
            std_string += "hello";
            assert_eq!(string, std_string);
        }

        #[test]
        fn test_add<S: Data<String>>(string: ImString<S>) {
            let std_string = string.as_str().to_string();
            let std_string = std_string + "hello";
            let string = string + "hello";
            assert_eq!(string, std_string);
        }

        #[test]
        fn test_to_socket_addrs<S: Data<String>>(string: ImString<S>) {
            
            #[cfg(all(not(miri), feature = "std"))]
            {
                let addrs = string.to_socket_addrs().map(|s| s.collect::<Vec<_>>());
                let str_addrs = string.as_str().to_socket_addrs().map(|s| s.collect::<Vec<_>>());
                match addrs {
                    Ok(addrs) => assert_eq!(addrs, str_addrs.unwrap()),
                    Err(_err) => assert!(str_addrs.is_err()),
                }
            }
        }

        #[test]
        fn test_from_iterator_char<S: Data<String>>() {
            let input = ['h', 'e', 'l', 'l', 'o'];
            let string: ImString<S> = input.into_iter().collect();
            assert_eq!(string, "hello");
        }

        #[test]
        fn test_from_iterator_char_ref<S: Data<String>>() {
            let input = ['h', 'e', 'l', 'l', 'o'];
            let string: ImString<S> = input.iter().collect();
            assert_eq!(string, "hello");
        }

        #[test]
        fn test_from_iterator_str<S: Data<String>>() {
            let input = ["hello", "world", "!"];
            let string: ImString<S> = input.into_iter().collect();
            assert_eq!(string, "helloworld!");
        }

        #[test]
        fn test_extend_char<S: Data<String>>() {
            let input = ['h', 'e', 'l', 'l', 'o'];
            let mut string: ImString<S> = ImString::new();
            string.extend(input.into_iter());
            assert_eq!(string, "hello");
        }

        #[test]
        fn test_extend_char_ref<S: Data<String>>() {
            let input = ['h', 'e', 'l', 'l', 'o'];
            let mut string: ImString<S> = ImString::new();
            string.extend(input.into_iter());
            assert_eq!(string, "hello");
        }

        #[test]
        fn test_extend_str<S: Data<String>>() {
            let input = ["hello", "world", "!"];
            let mut string: ImString<S> = ImString::new();
            string.extend(input.into_iter());
            assert_eq!(string, "helloworld!");
        }

        #[test]
        fn test_from_utf8_lossy<S: Data<String>>() {
            let string: ImString<S> = ImString::from_utf8_lossy(b"hello");
            assert_eq!(string, "hello");
        }

        #[test]
        fn test_from_utf8_unchecked<S: Data<String>>() {
            let string: ImString<S> = unsafe {
                ImString::from_utf8_unchecked(b"hello".to_vec())
            };
            assert_eq!(string, "hello");
        }

        #[test]
        fn test_as_ref_str<S: Data<String>>(string: ImString<S>) {
            let s: &str = string.as_ref();
            assert_eq!(s, string.as_str());
        }

        #[test]
        fn test_as_ref_bytes<S: Data<String>>(string: ImString<S>) {
            let s: &[u8] = string.as_ref();
            assert_eq!(s, string.as_bytes());
        }

        #[test]
        fn test_as_ref_path<S: Data<String>>(string: ImString<S>) {
            #[cfg(feature = "std")]
            {
                let s: &Path = string.as_ref();
                assert_eq!(s, string.as_str().as_ref() as &Path);
            }
        }

        #[test]
        fn test_as_ref_os_str<S: Data<String>>(string: ImString<S>) {
            #[cfg(feature = "std")]
            {
                let s: &OsStr = string.as_ref();
                assert_eq!(s, string.as_str().as_ref() as &OsStr);
            }
        }

        #[test]
        fn test_partial_eq<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string, string.as_str());
            assert_eq!(string, string.to_string());
            assert_eq!(string, string);
        }

        #[test]
        fn test_from<S: Data<String>>(string: ImString<S>) {
            let std_string: String = string.clone().into();
            assert_eq!(string, std_string);
        }

        #[test]
        fn test_raw_offset<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.offset, string.raw_offset());
        }

        #[test]
        fn test_raw_string<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string.string.get(), string.raw_string().get());
        }

        #[test]
        fn into_std_string<S: Data<String>>(string: ImString<S>) {
            let std_clone = string.as_str().to_string();
            let std_string = string.into_std_string();
            assert_eq!(std_clone, std_string);
        }

        #[test]
        fn test_truncate<S: Data<String>>(string: ImString<S>) {
            let mut clone = string.as_str().to_string();
            let mut string = string;

            for length in (0..string.len()).rev() {
                if string.is_char_boundary(length) {
                    string.truncate(length);
                    clone.truncate(length);
                    assert_eq!(string, clone);
                }
            }
        }

        #[test]
        fn test_str_ref<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string, string.str_ref(string.as_str()));
        }

        #[test]
        fn test_try_str_ref<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string, string.try_str_ref(string.as_str()).unwrap());
            assert_eq!(string.try_str_ref("test"), None);
        }

        #[test]
        fn test_slice_ref<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string, string.slice_ref(string.as_bytes()));
        }

        #[test]
        fn test_try_slice_ref<S: Data<String>>(string: ImString<S>) {
            assert_eq!(string, string.try_slice_ref(string.as_bytes()).unwrap());
            assert_eq!(string.try_slice_ref(b"test"), None);
        }
    }
}
