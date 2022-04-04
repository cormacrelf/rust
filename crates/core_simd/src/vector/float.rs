#![allow(non_camel_case_types)]

use crate::simd::intrinsics;
use crate::simd::{LaneCount, Mask, Simd, SimdPartialEq, SimdPartialOrd, SupportedLaneCount};

/// Implements inherent methods for a float vector containing multiple
/// `$lanes` of float `$type`, which uses `$bits_ty` as its binary
/// representation.
macro_rules! impl_float_vector {
    { $type:ty, $bits_ty:ty, $mask_ty:ty } => {
        impl<const LANES: usize> Simd<$type, LANES>
        where
            LaneCount<LANES>: SupportedLaneCount,
        {
            /// Raw transmutation to an unsigned integer vector type with the
            /// same size and number of lanes.
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn to_bits(self) -> Simd<$bits_ty, LANES> {
                assert_eq!(core::mem::size_of::<Self>(), core::mem::size_of::<Simd<$bits_ty, LANES>>());
                unsafe { core::mem::transmute_copy(&self) }
            }

            /// Raw transmutation from an unsigned integer vector type with the
            /// same size and number of lanes.
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn from_bits(bits: Simd<$bits_ty, LANES>) -> Self {
                assert_eq!(core::mem::size_of::<Self>(), core::mem::size_of::<Simd<$bits_ty, LANES>>());
                unsafe { core::mem::transmute_copy(&bits) }
            }

            /// Produces a vector where every lane has the absolute value of the
            /// equivalently-indexed lane in `self`.
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn abs(self) -> Self {
                unsafe { intrinsics::simd_fabs(self) }
            }

            /// Takes the reciprocal (inverse) of each lane, `1/x`.
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn recip(self) -> Self {
                Self::splat(1.0) / self
            }

            /// Converts each lane from radians to degrees.
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn to_degrees(self) -> Self {
                // to_degrees uses a special constant for better precision, so extract that constant
                self * Self::splat(<$type>::to_degrees(1.))
            }

            /// Converts each lane from degrees to radians.
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn to_radians(self) -> Self {
                self * Self::splat(<$type>::to_radians(1.))
            }

            /// Returns true for each lane if it has a positive sign, including
            /// `+0.0`, `NaN`s with positive sign bit and positive infinity.
            #[inline]
            #[must_use = "method returns a new mask and does not mutate the original value"]
            pub fn is_sign_positive(self) -> Mask<$mask_ty, LANES> {
                !self.is_sign_negative()
            }

            /// Returns true for each lane if it has a negative sign, including
            /// `-0.0`, `NaN`s with negative sign bit and negative infinity.
            #[inline]
            #[must_use = "method returns a new mask and does not mutate the original value"]
            pub fn is_sign_negative(self) -> Mask<$mask_ty, LANES> {
                let sign_bits = self.to_bits() & Simd::splat((!0 >> 1) + 1);
                sign_bits.simd_gt(Simd::splat(0))
            }

            /// Returns true for each lane if its value is `NaN`.
            #[inline]
            #[must_use = "method returns a new mask and does not mutate the original value"]
            pub fn is_nan(self) -> Mask<$mask_ty, LANES> {
                self.simd_ne(self)
            }

            /// Returns true for each lane if its value is positive infinity or negative infinity.
            #[inline]
            #[must_use = "method returns a new mask and does not mutate the original value"]
            pub fn is_infinite(self) -> Mask<$mask_ty, LANES> {
                self.abs().simd_eq(Self::splat(<$type>::INFINITY))
            }

            /// Returns true for each lane if its value is neither infinite nor `NaN`.
            #[inline]
            #[must_use = "method returns a new mask and does not mutate the original value"]
            pub fn is_finite(self) -> Mask<$mask_ty, LANES> {
                self.abs().simd_lt(Self::splat(<$type>::INFINITY))
            }

            /// Returns true for each lane if its value is subnormal.
            #[inline]
            #[must_use = "method returns a new mask and does not mutate the original value"]
            pub fn is_subnormal(self) -> Mask<$mask_ty, LANES> {
                self.abs().simd_ne(Self::splat(0.0)) & (self.to_bits() & Self::splat(<$type>::INFINITY).to_bits()).simd_eq(Simd::splat(0))
            }

            /// Returns true for each lane if its value is neither zero, infinite,
            /// subnormal, nor `NaN`.
            #[inline]
            #[must_use = "method returns a new mask and does not mutate the original value"]
            pub fn is_normal(self) -> Mask<$mask_ty, LANES> {
                !(self.abs().simd_eq(Self::splat(0.0)) | self.is_nan() | self.is_subnormal() | self.is_infinite())
            }

            /// Replaces each lane with a number that represents its sign.
            ///
            /// * `1.0` if the number is positive, `+0.0`, or `INFINITY`
            /// * `-1.0` if the number is negative, `-0.0`, or `NEG_INFINITY`
            /// * `NAN` if the number is `NAN`
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn signum(self) -> Self {
                self.is_nan().select(Self::splat(<$type>::NAN), Self::splat(1.0).copysign(self))
            }

            /// Returns each lane with the magnitude of `self` and the sign of `sign`.
            ///
            /// If any lane is a `NAN`, then a `NAN` with the sign of `sign` is returned.
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            pub fn copysign(self, sign: Self) -> Self {
                let sign_bit = sign.to_bits() & Self::splat(-0.).to_bits();
                let magnitude = self.to_bits() & !Self::splat(-0.).to_bits();
                Self::from_bits(sign_bit | magnitude)
            }
        }
    };
}

impl_float_vector! { f32, u32, i32 }
impl_float_vector! { f64, u64, i64 }

/// A 64-bit SIMD vector with two elements of type `f32`.
pub type f32x2 = Simd<f32, 2>;

/// A 128-bit SIMD vector with four elements of type `f32`.
pub type f32x4 = Simd<f32, 4>;

/// A 256-bit SIMD vector with eight elements of type `f32`.
pub type f32x8 = Simd<f32, 8>;

/// A 512-bit SIMD vector with 16 elements of type `f32`.
pub type f32x16 = Simd<f32, 16>;

/// A 128-bit SIMD vector with two elements of type `f64`.
pub type f64x2 = Simd<f64, 2>;

/// A 256-bit SIMD vector with four elements of type `f64`.
pub type f64x4 = Simd<f64, 4>;

/// A 512-bit SIMD vector with eight elements of type `f64`.
pub type f64x8 = Simd<f64, 8>;

mod sealed {
    pub trait Sealed {}
}
use sealed::Sealed;

/// SIMD operations on vectors of floating point numbers.
pub trait SimdFloat: Sized + Sealed {
    /// Returns the minimum of each lane.
    ///
    /// If one of the values is `NAN`, then the other value is returned.
    #[must_use = "method returns a new vector and does not mutate the original value"]
    fn simd_min(self, other: Self) -> Self;

    /// Returns the maximum of each lane.
    ///
    /// If one of the values is `NAN`, then the other value is returned.
    #[must_use = "method returns a new vector and does not mutate the original value"]
    fn simd_max(self, other: Self) -> Self;

    /// Restrict each lane to a certain interval unless it is NaN.
    ///
    /// For each lane in `self`, returns the corresponding lane in `max` if the lane is
    /// greater than `max`, and the corresponding lane in `min` if the lane is less
    /// than `min`.  Otherwise returns the lane in `self`.
    #[must_use = "method returns a new vector and does not mutate the original value"]
    fn simd_clamp(self, min: Self, max: Self) -> Self;
}

macro_rules! impl_simd_float {
    { $($float:ty),* } => {
        $(
        impl <const LANES: usize> Sealed for Simd<$float, LANES>
        where
            LaneCount<LANES>: SupportedLaneCount,
        {
        }

        impl <const LANES: usize> SimdFloat for Simd<$float, LANES>
        where
            LaneCount<LANES>: SupportedLaneCount,
        {
            #[inline]
            #[must_use = "method returns a new vector and does not mutate the original value"]
            fn simd_min(self, other: Self) -> Self {
                unsafe { intrinsics::simd_fmin(self, other) }
            }

            #[inline]
            fn simd_max(self, other: Self) -> Self {
                unsafe { intrinsics::simd_fmax(self, other) }
            }

            #[inline]
            fn simd_clamp(self, min: Self, max: Self) -> Self {
                assert!(
                    min.simd_le(max).all(),
                    "each lane in `min` must be less than or equal to the corresponding lane in `max`",
                );
                let mut x = self;
                x = x.simd_lt(min).select(min, x);
                x = x.simd_gt(max).select(max, x);
                x
            }
        }
        )*
    }
}

impl_simd_float! { f32, f64 }
