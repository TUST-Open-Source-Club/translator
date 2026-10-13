# Generic x86_64 releases use the SSE2 architectural baseline. GGML_NATIVE=OFF
# alone still enables AVX2/FMA/F16C in llama.cpp's default options. Do not force
# these settings into local builds or arm64 (Metal/Android) release jobs.
foreach(feature NATIVE SSE42 AVX AVX2 BMI2 FMA F16C AVX_VNNI AVX512
        AVX512_VBMI AVX512_VNNI AVX512_BF16 AMX_TILE AMX_INT8 AMX_BF16)
    set(GGML_${feature} OFF CACHE BOOL "Generic x86_64 release baseline" FORCE)
endforeach()
