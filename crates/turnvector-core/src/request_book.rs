#![allow(dead_code, reason = "later C11 rows consume bounded token requests")]

use crate::model_registry::{ModelAliasId, ModelRevisionId};
use crate::{BoundedVec, ServiceClass, TokenCount};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestSelector {
    Direct(ModelRevisionId),
    Alias(ModelAliasId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SamplingMode {
    Greedy,
    Categorical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SamplingSeedOrigin {
    Caller,
    Daemon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveSamplingSeed {
    value: u64,
    origin: SamplingSeedOrigin,
}

impl EffectiveSamplingSeed {
    #[rustfmt::skip]
    pub(crate) const fn new(value: u64, origin: SamplingSeedOrigin) -> Self { Self { value, origin } }
    #[rustfmt::skip]
    pub(crate) const fn value(self) -> u64 { self.value }
    #[rustfmt::skip]
    pub(crate) const fn origin(self) -> SamplingSeedOrigin { self.origin }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationParameters {
    mode: SamplingMode,
    temperature_bits: u32,
    top_p_bits: u32,
    top_k: u32,
}

impl GenerationParameters {
    pub(crate) fn try_new(
        mode: SamplingMode,
        temperature_bits: u32,
        top_p_bits: u32,
        top_k: u32,
    ) -> Result<Self, RequestError> {
        let (temperature, top_p) = (f32::from_bits(temperature_bits), f32::from_bits(top_p_bits));
        let valid = match mode {
            SamplingMode::Greedy => {
                temperature_bits == 0 && top_p_bits == 1.0f32.to_bits() && top_k == 0
            }
            SamplingMode::Categorical => {
                temperature.is_finite()
                    && temperature > 0.0
                    && temperature <= 2.0
                    && top_p.is_finite()
                    && top_p > 0.0
                    && top_p <= 1.0
            }
        };
        valid
            .then_some(Self {
                mode,
                temperature_bits,
                top_p_bits,
                top_k,
            })
            .ok_or(RequestError::GenerationParameters)
    }

    #[rustfmt::skip]
    pub(crate) const fn mode(self) -> SamplingMode { self.mode }
    #[rustfmt::skip]
    pub(crate) const fn temperature_bits(self) -> u32 { self.temperature_bits }
    #[rustfmt::skip]
    pub(crate) const fn top_p_bits(self) -> u32 { self.top_p_bits }
    #[rustfmt::skip]
    pub(crate) const fn top_k(self) -> u32 { self.top_k }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestError {
    GenerationParameters,
    InputTokenCapacity,
    MaxOutputTokens,
    StopSequenceCapacity,
    EmptyStopTokenSequence,
    StopTokenCapacity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TokenRequest<const INPUT: usize, const STOPS: usize, const STOP_TOKENS: usize> {
    selector: RequestSelector,
    input: BoundedVec<u32, INPUT>,
    parameters: GenerationParameters,
    service: ServiceClass,
    max_output: TokenCount,
    stops: BoundedVec<BoundedVec<u32, STOP_TOKENS>, STOPS>,
    seed: EffectiveSamplingSeed,
}

impl<const I: usize, const S: usize, const T: usize> TokenRequest<I, S, T> {
    pub(crate) fn try_new(
        selector: RequestSelector,
        input: &[u32],
        parameters: GenerationParameters,
        service: ServiceClass,
        max_output: TokenCount,
        stops: &[&[u32]],
        seed: EffectiveSamplingSeed,
    ) -> Result<Self, RequestError> {
        if max_output.get() == 0 {
            return Err(RequestError::MaxOutputTokens);
        }
        let input = bounded(input, RequestError::InputTokenCapacity)?;
        let mut retained_stops = BoundedVec::new();
        for &stop in stops {
            if stop.is_empty() {
                return Err(RequestError::EmptyStopTokenSequence);
            }
            retained_stops
                .try_push(bounded(stop, RequestError::StopTokenCapacity)?)
                .map_err(|_| RequestError::StopSequenceCapacity)?;
        }
        Ok(Self {
            selector,
            input,
            parameters,
            service,
            max_output,
            stops: retained_stops,
            seed,
        })
    }

    #[rustfmt::skip]
    pub(crate) const fn selector(&self) -> RequestSelector { self.selector }
    #[rustfmt::skip]
    pub(crate) const fn input(&self) -> &BoundedVec<u32, I> { &self.input }
    #[rustfmt::skip]
    pub(crate) const fn parameters(&self) -> GenerationParameters { self.parameters }
    #[rustfmt::skip]
    pub(crate) const fn service(&self) -> ServiceClass { self.service }
    #[rustfmt::skip]
    pub(crate) const fn max_output(&self) -> TokenCount { self.max_output }
    #[rustfmt::skip]
    pub(crate) const fn stops(&self) -> &BoundedVec<BoundedVec<u32, T>, S> { &self.stops }
    #[rustfmt::skip]
    pub(crate) const fn seed(&self) -> EffectiveSamplingSeed { self.seed }
}

#[rustfmt::skip]
fn bounded<const N: usize>(values: &[u32], error: RequestError) -> Result<BoundedVec<u32, N>, RequestError> {
    let mut bounded = BoundedVec::new();
    for &value in values {
        bounded.try_push(value).map_err(|_| error)?;
    }
    Ok(bounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_registry::{ModelAliasId, ModelRevisionId};
    use crate::{ServiceClass, TokenCount};

    #[rustfmt::skip]
    fn greedy() -> GenerationParameters { GenerationParameters::try_new(SamplingMode::Greedy, 0.0f32.to_bits(), 1.0f32.to_bits(), 0).unwrap() }

    #[test]
    #[rustfmt::skip]
    fn selectors_and_values_are_closed_and_retained() {
        let revision = ModelRevisionId::new([1; 32]).unwrap();
        let direct = TokenRequest::<2, 1, 2>::try_new(RequestSelector::Direct(revision), &[11, 12], greedy(), ServiceClass::Interactive, TokenCount::new(3), &[], EffectiveSamplingSeed::new(0, SamplingSeedOrigin::Caller)).unwrap();
        assert_eq!((direct.selector(), direct.input().len(), direct.seed()), (RequestSelector::Direct(revision), 2, EffectiveSamplingSeed::new(0, SamplingSeedOrigin::Caller)));
        let alias = ModelAliasId::new([2; 32]).unwrap();
        let parameters = GenerationParameters::try_new(SamplingMode::Categorical, 0.5f32.to_bits(), 0.9f32.to_bits(), 7).unwrap();
        let request = TokenRequest::<3, 2, 3>::try_new(RequestSelector::Alias(alias), &[1, 2, 3], parameters, ServiceClass::Background, TokenCount::new(4), &[&[5], &[6, 7]], EffectiveSamplingSeed::new(u64::MAX, SamplingSeedOrigin::Daemon)).unwrap();
        assert_eq!((request.selector(), request.parameters(), request.service(), request.max_output().get(), request.seed()), (RequestSelector::Alias(alias), parameters, ServiceClass::Background, 4, EffectiveSamplingSeed::new(u64::MAX, SamplingSeedOrigin::Daemon)));
        assert_eq!(request.stops().get(1).unwrap().iter().copied().collect::<Vec<_>>(), vec![6, 7]);
    }

    #[test]
    #[rustfmt::skip]
    fn generation_parameters_enforce_exact_binary32_domains() {
        let minimum = GenerationParameters::try_new(SamplingMode::Categorical, 1, 1, u32::MAX).unwrap();
        assert_eq!((minimum.mode(), minimum.temperature_bits(), minimum.top_p_bits(), minimum.top_k()), (SamplingMode::Categorical, 1, 1, u32::MAX));
        for (mode, temperature, top_p, top_k) in [
            (SamplingMode::Greedy, (-0.0f32).to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Greedy, 0.0f32.to_bits(), 0.5f32.to_bits(), 0),
            (SamplingMode::Greedy, 0.0f32.to_bits(), 1.0f32.to_bits(), 1),
            (SamplingMode::Categorical, 0.0f32.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, (-0.0f32).to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, f32::NAN.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, f32::INFINITY.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, f32::NEG_INFINITY.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, 2.0f32.to_bits() + 1, 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), (-0.0f32).to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), f32::NAN.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), f32::INFINITY.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), f32::NEG_INFINITY.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), 1.0f32.to_bits() + 1, 0),
        ] {
            assert_eq!(GenerationParameters::try_new(mode, temperature, top_p, top_k), Err(RequestError::GenerationParameters));
        }
        GenerationParameters::try_new(SamplingMode::Categorical, 2.0f32.to_bits(), 1.0f32.to_bits(), 0).unwrap();
    }

    #[rustfmt::skip]
    fn bounded_request(input: &[u32], output: u64, stops: &[&[u32]]) -> Result<TokenRequest<2, 2, 2>, RequestError> { TokenRequest::try_new(RequestSelector::Direct(ModelRevisionId::new([3; 32]).unwrap()), input, greedy(), ServiceClass::Standard, TokenCount::new(output), stops, EffectiveSamplingSeed::new(u64::MAX, SamplingSeedOrigin::Caller)) }

    #[test]
    #[rustfmt::skip]
    fn token_and_stop_bounds_reject_without_truncation() {
        let exact = bounded_request(&[1, 2], 1, &[&[3, 4], &[5]]).unwrap();
        assert_eq!((exact.input().len(), exact.stops().len(), exact.seed().value()), (2, 2, u64::MAX));
        assert!(bounded_request(&[], 1, &[]).is_ok());
        assert_eq!(bounded_request(&[1, 2, 3], 1, &[]), Err(RequestError::InputTokenCapacity));
        assert_eq!(bounded_request(&[], 0, &[]), Err(RequestError::MaxOutputTokens));
        assert_eq!(bounded_request(&[], 1, &[&[1], &[2], &[3]]), Err(RequestError::StopSequenceCapacity));
        assert_eq!(bounded_request(&[], 1, &[&[]]), Err(RequestError::EmptyStopTokenSequence));
        assert_eq!(bounded_request(&[], 1, &[&[1, 2, 3]]), Err(RequestError::StopTokenCapacity));
    }
}
