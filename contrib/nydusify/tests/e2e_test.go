// Copyright 2020 Ant Group. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

package tests

import (
	"testing"
)

func TestBasicConvert(t *testing.T) {
	registry := NewRegistry(t)
	defer registry.Destory(t)

	registry.Build(t, "image-basic")
	nydusify := NewNydusify(registry, "image-basic", "image-basic-nydus", "")
	nydusify.Convert(t)
	nydusify.Check(t)
}

func TestConvertWithCache(t *testing.T) {
	registry := NewRegistry(t)
	defer registry.Destory(t)

	registry.Build(t, "image-basic")
	nydusify1 := NewNydusify(registry, "image-basic", "image-basic-nydus-1", "cache:v1")
	nydusify1.Convert(t)
	nydusify1.Check(t)

	nydusify2 := NewNydusify(registry, "image-basic", "image-basic-nydus-2", "cache:v1")
	nydusify2.Convert(t)
	nydusify2.Check(t)

	registry.Build(t, "image-from")
	nydusify3 := NewNydusify(registry, "image-from", "image-from-nydus-1", "cache:v1")
	nydusify3.Convert(t)
	nydusify3.Check(t)
}
