/**
 * AI features repository — face/object/pet clusters and global AI controls.
 */
package com.simplephotos.data.repository

import com.simplephotos.data.remote.ApiService
import com.simplephotos.data.remote.dto.*
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AiRepository @Inject constructor(private val api: ApiService) {

    suspend fun status(): AiStatusResponse = api.getAiStatus()

    suspend fun toggle(enabled: Boolean) {
        api.toggleAi(AiToggleRequest(enabled))
    }

    suspend fun reprocess(photoIds: List<String>? = null): AiReprocessResponse =
        api.reprocessAi(AiReprocessRequest(photoIds))

    // Face clusters
    suspend fun listFaceClusters(): List<FaceCluster> =
        api.listFaceClusters()

    suspend fun mergeFaceClusters(clusterIds: List<Long>) {
        api.mergeFaceClusters(FaceClusterMergeRequest(clusterIds))
    }

    suspend fun splitFaceCluster(detectionIds: List<Long>) {
        api.splitFaceCluster(FaceClusterSplitRequest(detectionIds))
    }

    suspend fun listFaceClusterPhotos(clusterId: String): List<FaceClusterPhotoEntry> =
        api.listFaceClusterPhotos(clusterId)

    suspend fun renameFaceCluster(clusterId: String, name: String) {
        api.renameFaceCluster(clusterId, FaceClusterRenameRequest(name))
    }

    // Object classes
    suspend fun listObjectClasses(): List<ObjectClass> =
        api.listObjectClasses()

    suspend fun listObjectClassPhotos(className: String): List<ObjectClassPhotoEntry> =
        api.listObjectClassPhotos(className)

    // Pet clusters
    suspend fun listPetClusters(): List<PetCluster> =
        api.listPetClusters()

    suspend fun mergePetClusters(clusterIds: List<Long>) {
        api.mergePetClusters(PetClusterMergeRequest(clusterIds))
    }

    suspend fun listPetClusterPhotos(clusterId: String): List<PetClusterPhotoEntry> =
        api.listPetClusterPhotos(clusterId)

    suspend fun renamePetCluster(clusterId: String, name: String) {
        api.renamePetCluster(clusterId, PetClusterRenameRequest(name))
    }
}
